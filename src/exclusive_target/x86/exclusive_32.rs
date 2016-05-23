/// Like mem::epoch::AtomicPtr, but provides an ll/sc based api on x86, powerpc, arm, aarch64

use std::mem;
use std::marker::PhantomData;

use std::sync::atomic::{Ordering, AtomicUsize};

#[repr(C)]
#[derive(Copy, Clone)]
struct Llsc {
    val: u32,
    counter: u32,
    extra: u32, //we adjust which is actually the real one due to alignment
}

#[inline(always)]
unsafe fn load_from(ptr: *const u32, ord: Ordering) -> u32 {
    let ptr: *const AtomicUsize = mem::transmute(ptr);
    (&*ptr).load(ord) as u32
}

#[inline(always)]
unsafe fn store_to(ptr: *const u32, n: u32, ord: Ordering) {
    let ptr: *const AtomicUsize = mem::transmute(ptr);
    (&*ptr).store(n as usize, ord)
}

#[inline(always)]
unsafe fn exchange_to(ptr: *const u32, n: u32, ord: Ordering) -> u32 {
    let ptr: *const AtomicUsize = mem::transmute(ptr);
    (&*ptr).swap(n as usize, ord) as u32
}


#[inline(always)]
unsafe fn cas_tagged(ptr: *const u32, old: (u32, u32), nval: u32) -> (bool, (u32, u32)) {
    let mut val: u32 = old.0;
    let mut counter: u32 = old.1;
    let ncounter: u32 = counter.wrapping_add(1);
    let new = nval;
    let succ: bool;
    asm!("lock cmpxchg8b ($7)\n\t
          sete $0\n\t"
         : "=r" (succ), "={eax}" (val), "={edx}" (counter)
         : "1"(val), "2"(counter), "{ebx}"(new), "{ecx}"(ncounter), "r"(ptr)
         : "memory"
         : "volatile");
    // Returned values only matter if succ is false,
    // in which case thee right ones are loaded into memory
    (succ, (val, counter))
}

// All this junk is to ensure 8-byte alignment
// generally working alignment specifiers would solve this and some
// effecient issues, although those will be dominated by cas
impl Llsc {
    pub unsafe fn get_ptr(&self) -> *const u32 {
        let addr: *const u32 = &self.counter;
        let addr64 = addr as u32;
        (addr64 & !7) as *const u32 // if &val is 8 byte aligned, will round down
        // otherwise, will keep address of counter
    }

    pub unsafe fn get_vals(&self, ord: Ordering) -> (u32, u32) {
        let ptr = self.get_ptr();
        (load_from(ptr, ord), *ptr.offset(1))
    }

    pub unsafe fn set_val(&self, val: u32, ord: Ordering) {
        store_to(self.get_ptr(), val, ord);
    }

    pub unsafe fn xchg_val(&self, val: u32, ord: Ordering) -> u32 {
        exchange_to(self.get_ptr(), val, ord)
    }
}

pub trait Isu32 {
    fn from_u32(val: u32) -> Self;
    fn to_u32(&self) -> u32;
}

impl Isu32 for usize {
    fn from_u32(val: u32) -> usize {
        val as usize
    }

    fn to_u32(&self) -> u32 {
        *self as u32
    }
}

impl Isu32 for isize {
    fn from_u32(val: u32) -> isize {
        val as isize
    }

    fn to_u32(&self) -> u32 {
        *self as u32
    }
}

impl<T> Isu32 for *mut T {

    fn from_u32(val: u32) -> *mut T {
        val as *mut T
    }

    fn to_u32(&self) -> u32 {
        *self as u32
    }
}

impl Isu32 for bool {

    fn from_u32(val: u32) -> bool {
        val == 0
    }

    fn to_u32(&self) -> u32 {
        *self as u32
    }
}

pub struct ExclusiveData<T: Isu32> {
    data: Llsc,
    marker: PhantomData<T>,
}

pub struct LinkedData<'a, T: 'a + Isu32> {
    data: (u32, u32),
    ptr: *const u32,
    _borrowck: &'a ExclusiveData<T>,
}

impl<T: Isu32> ExclusiveData<T> {

    pub fn new(val: T) -> ExclusiveData<T> {
        ExclusiveData {
            data: Llsc {
                val: val.to_u32(),
                counter: val.to_u32(),
                extra: 0,
            },
            marker: PhantomData,
        }
    }

    /// Loads the value from the pointer with the given ordering
    pub fn load(&self, ord: Ordering) -> T {
        unsafe { T::from_u32(self.data.get_vals(ord).0) }
    }

    /// Stores directly to the pointer without updating the counter
    ///
    /// This function can still leave one vulnerable to the ABA problem,
    /// But is useful when only used to store to say a null value.
    /// Be careful when using, this must always cause a store_conditional to fail
    pub fn store_direct(&self, val: T, ord: Ordering) {
        unsafe { self.data.set_val(val.to_u32(), ord) };
    }

    /// Stores directly to the pointer without updating the counter
    ///
    /// This function can still leave one vulnerable to the ABA problem,
    /// But is useful when only used to store to say a null value.
    /// Be careful when using, this must always cause a store_conditional to fail
    pub fn exchange_direct(&self, val: T, ord: Ordering) -> T {
        unsafe { T::from_u32(self.data.xchg_val(val.to_u32(), ord)) }
    }

    /// Performs an exclusive load on the pointer
    ///
    /// If the pointer is modified by a different store_conditional in between the load_linked
    /// and store_conditional, this will always fail. This is stronger the cas
    /// since cas can succedd when modifications have occured as long as the end
    /// result is the same. However, this will always fail in a scenario where cas would fail.
    pub fn load_linked(&self, ord: Ordering) -> LinkedData<T> {
        unsafe {
            LinkedData {
                data: self.data.get_vals(ord),
                ptr: self.data.get_ptr(),
                _borrowck: self,
            }
        }
    }
}

impl<'a, T: Isu32> LinkedData<'a, T> {

    pub fn get(&self) -> T {
        T::from_u32(self.data.0)
    }

    /// Performs a conditional store on the pointer, conditional on no modifications occurring
    ///
    /// If the pointer is modified by a different store_conditional in between the load_linked
    /// and store_conditional, this will always fail. This is stronger the cas
    /// since cas can succedd when modifications have occured as long as the end
    /// result is the same. However, this will always fail in a scenario where cas would fail.
    pub fn store_conditional(self, val: T, _: Ordering) -> Option<LinkedData<'a, T>> {
        unsafe {
            let (succ, res) = cas_tagged(self.ptr, self.data, val.to_u32());
            match succ {
                true => None,
                false => Some(LinkedData {
                    data: res,
                    ptr: self.ptr,
                    _borrowck: self._borrowck,
                })
            }
        }
    }
}

unsafe impl<T: Isu32> Send for ExclusiveData<T> {}
unsafe impl<T: Isu32> Sync for ExclusiveData<T> {}

pub type ExclusivePtr<T> = ExclusiveData<*mut T>;
pub type ExclusiveUsize = ExclusiveData<usize>;
pub type ExclusiveIsize = ExclusiveData<isize>;

// This could be more efficient, by doing normal cas and packing
// as u32. BUT! That's code bloat for the time being
pub type ExclusiveBool = ExclusiveData<bool>;

pub type LinkedPtr<'a, T> = LinkedData<'a, *mut T>;
pub type LinkedUsize<'a> = LinkedData<'a, usize>;
pub type LinkedIsize<'a> = LinkedData<'a, isize>;
pub type LinkedBool<'a> = LinkedData<'a, bool>;

#[cfg(test)]
mod test {
    use scope;
    use super::*;
    use std::ptr;
    use std::sync::atomic::Ordering::{Relaxed};
    #[test]
    fn test_cas () {
        let mut val: u32 = 0;
        let eptr = ExclusivePtr::<u32>::new(ptr::null_mut());
        let ll = eptr.load_linked(Relaxed);
        assert_eq!(eptr.load(Relaxed), ptr::null_mut());
        assert_eq!(ll.store_conditional(&mut val, Relaxed).is_none(), true);
        assert_eq!(eptr.load(Relaxed), &mut val as *mut u32);
    }

    #[test]
    fn test_cas_fail () {
        let mut val: u32 = 0;
        let mut val2: u32 = 0;
        let eptr = ExclusivePtr::<u32>::new(ptr::null_mut());
        let ll = eptr.load_linked(Relaxed);
        assert_eq!(eptr.load(Relaxed), ptr::null_mut());
        eptr.store_direct(&mut val2, Relaxed);
        assert_eq!(eptr.load(Relaxed), &mut val2 as *mut u32);
        assert_eq!(ll.store_conditional(&mut val, Relaxed).is_some(), true);
        assert_eq!(eptr.load(Relaxed), &mut val2 as *mut u32);
    }

    #[test]
    fn test_cas_fail_xchg () {
        let mut val: u32 = 0;
        let mut val2: u32 = 0;
        let eptr = ExclusivePtr::<u32>::new(ptr::null_mut());
        let ll = eptr.load_linked(Relaxed);
        assert_eq!(eptr.load(Relaxed), ptr::null_mut());
        assert_eq!(eptr.exchange_direct(&mut val2, Relaxed), ptr::null_mut());
        assert_eq!(eptr.load(Relaxed), &mut val2 as *mut u32);
        assert_eq!(ll.store_conditional(&mut val, Relaxed).is_some(), true);
        assert_eq!(eptr.load(Relaxed), &mut val2 as *mut u32);
    }

    #[test]
    fn test_mt_cas() {
        let num_run: usize = 1000000;
        let num_thread: usize = 4;
        let val = ExclusiveUsize::new(0);

        scope(|scope| {
            for _ in 0..num_thread {
                scope.spawn(||{
                    for _ in 0..num_run {
                        let mut ll = val.load_linked(Relaxed);
                        loop {
                            let next = ll.get() + 1;
                            match ll.store_conditional(next, Relaxed) {
                                None => break,
                                Some(nll) => ll = nll,
                            }
                        }
                    }
                });
            }
        });

        assert_eq!(val.load(Relaxed), num_run * num_thread);
    }
}
