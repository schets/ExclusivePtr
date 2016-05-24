/// Like mem::epoch::AtomicPtr, but provides an ll/sc based api on x86, powerpc, arm, aarch64

use std::mem;
use std::marker::PhantomData;

use std::sync::atomic::{Ordering, AtomicUsize};

#[cfg(target_pointer_width = "64")]
mod multi_size {

    pub const PTR_MOD: usize = 15;

    #[inline(always)]
    pub unsafe fn cas_tagged(ptr: *const usize, old: (usize, usize), nval: usize)
                         -> (bool, (usize, usize)) {
        let mut val: usize = old.0;
        let mut counter: usize = old.1;
        let ncounter: usize = counter.wrapping_add(1);
        let new = nval;
        let succ: bool;
        asm!("lock cmpxchg16b ($7)\n\t
          sete $0\n\t"
          : "=r" (succ), "={rax}" (val), "={rdx}" (counter)
          : "1"(val), "2"(counter), "{rbx}"(new), "{rcx}"(ncounter), "r"(ptr)
          : "memory"
          : "volatile");
        // Returned values only matter if succ is false,
        // in which case thee right ones are loaded into memory
        (succ, (val, counter))
    }
}

#[cfg(target_pointer_width = "32")]
mod multi_size {

    pub const PRT_MOD: usize = 7;

    #[inline(always)]
    pub unsafe fn cas_tagged(ptr: *const usize, old: (usize, usize), nval: usize)
                         -> (bool, (usize, usize)) {
        let mut val: usize = old.0;
        let mut counter: usize = old.1;
        let ncounter: usize = counter.wrapping_add(1);
        let new = nval;
        let succ: bool;
        asm!("lock cmpxchg16b ($7)\n\t
          sete $0\n\t"
          : "=r" (succ), "={rax}" (val), "={rdx}" (counter)
          : "1"(val), "2"(counter), "{rbx}"(new), "{rcx}"(ncounter), "r"(ptr)
          : "memory"
          : "volatile");
        // Returned values only matter if succ is false,
        // in which case thee right ones are loaded into memory
        (succ, (val, counter))
    }
}

use self::multi_size::*;


#[repr(C)]
#[derive(Copy, Clone)]
struct Llsc {
    val: usize,
    counter: usize,
    extra: usize, //we adjust which is actually the real one due to alignment
}

#[inline(always)]
unsafe fn load_from(ptr: *const usize, ord: Ordering) -> usize {
    let ptr: *const AtomicUsize = mem::transmute(ptr);
    (&*ptr).load(ord)
}

#[inline(always)]
unsafe fn store_to(ptr: *const usize, n: usize, ord: Ordering) {
    let ptr: *const AtomicUsize = mem::transmute(ptr);
    (&*ptr).store(n, ord)
}

#[inline(always)]
unsafe fn exchange_to(ptr: *const usize, n: usize, ord: Ordering) -> usize {
    let ptr: *const AtomicUsize = mem::transmute(ptr);
    (&*ptr).swap(n, ord)
}

impl Llsc {
    pub unsafe fn get_ptr(&self) -> *const usize {
        let addr = (&self.counter as *const usize) as usize;
        (addr & !PTR_MOD) as *const usize
    }

    pub unsafe fn get_vals(&self, ord: Ordering) -> (usize, usize) {
        let ptr = self.get_ptr();
        (load_from(ptr, ord), *ptr.offset(1))
    }

    pub unsafe fn set_val(&self, val: usize, ord: Ordering) {
        store_to(self.get_ptr(), val, ord);
    }

    pub unsafe fn xchg_val(&self, val: usize, ord: Ordering) -> usize {
        exchange_to(self.get_ptr(), val, ord)
    }
}

pub trait Isusize {
    fn from_usize(val: usize) -> Self;
    fn to_usize(&self) -> usize;
}

impl Isusize for usize {
    fn from_usize(val: usize) -> usize {
        val as usize
    }

    fn to_usize(&self) -> usize {
        *self as usize
    }
}

impl Isusize for isize {
    fn from_usize(val: usize) -> isize {
        val as isize
    }

    fn to_usize(&self) -> usize {
        *self as usize
    }
}

impl<T> Isusize for *mut T {

    fn from_usize(val: usize) -> *mut T {
        val as *mut T
    }

    fn to_usize(&self) -> usize {
        *self as usize
    }
}

impl Isusize for bool {

    fn from_usize(val: usize) -> bool {
        val == 0
    }

    fn to_usize(&self) -> usize {
        *self as usize
    }
}

pub struct ExclusiveData<T: Isusize> {
    data: Llsc,
    marker: PhantomData<T>,
}

pub struct LinkedData<'a, T: 'a + Isusize> {
    data: (usize, usize),
    ptr: *const usize,
    _borrowck: &'a ExclusiveData<T>,
}

impl<T: Isusize> ExclusiveData<T> {

    pub fn new(val: T) -> ExclusiveData<T> {
        ExclusiveData {
            data: Llsc {
                val: val.to_usize(),
                counter: val.to_usize(),
                extra: 0,
            },
            marker: PhantomData,
        }
    }

    /// Loads the value from the pointer with the given ordering
    pub fn load(&self, ord: Ordering) -> T {
        unsafe { T::from_usize(self.data.get_vals(ord).0) }
    }

    /// Stores directly to the pointer without updating the counter
    ///
    /// This function can still leave one vulnerable to the ABA problem,
    /// But is useful when only used to store to say a null value.
    /// Be careful when using, this must always cause a store_conditional to fail
    pub fn store_direct(&self, val: T, ord: Ordering) {
        unsafe { self.data.set_val(val.to_usize(), ord) };
    }

    /// Stores directly to the pointer without updating the counter
    ///
    /// This function can still leave one vulnerable to the ABA problem,
    /// But is useful when only used to store to say a null value.
    /// Be careful when using, this must always cause a store_conditional to fail
    pub fn exchange_direct(&self, val: T, ord: Ordering) -> T {
        unsafe { T::from_usize(self.data.xchg_val(val.to_usize(), ord)) }
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

impl<'a, T: Isusize> LinkedData<'a, T> {

    pub fn get(&self) -> T {
        T::from_usize(self.data.0)
    }

    /// Performs a conditional store on the pointer, conditional on no modifications occurring
    ///
    /// If the pointer is modified by a different store_conditional in between the load_linked
    /// and store_conditional, this will always fail. This is stronger the cas
    /// since cas can succedd when modifications have occured as long as the end
    /// result is the same. However, this will always fail in a scenario where cas would fail.
    pub fn store_conditional(self, val: T, _: Ordering) -> Option<LinkedData<'a, T>> {
        unsafe {
            let (succ, res) = cas_tagged(self.ptr, self.data, val.to_usize());
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

    /// Performs a conditional store on the pointer, conditional on no modifications occurring
    ///
    /// If the pointer is modified by a different store_conditional in between the load_linked
    /// and store_conditional, this will always fail. This is stronger the cas
    /// since cas can succedd when modifications have occured as long as the end
    /// result is the same. However, this will always fail in a scenario where cas would fail.
    pub fn try_store_conditional(self, val: T, _: Ordering) -> bool {
        unsafe {
            cas_tagged(self.ptr, self.data, val.to_usize()).0
        }
    }
}

unsafe impl<T: Isusize> Send for ExclusiveData<T> {}
unsafe impl<T: Isusize> Sync for ExclusiveData<T> {}

pub type ExclusivePtr<T> = ExclusiveData<*mut T>;
pub type ExclusiveUsize = ExclusiveData<usize>;
pub type ExclusiveIsize = ExclusiveData<isize>;

// This could be more efficient, by doing normal cas and packing
// as usize. BUT! That's code bloat for the time being
pub type ExclusiveBool = ExclusiveData<bool>;

pub type LinkedPtr<'a, T> = LinkedData<'a, *mut T>;
pub type LinkedUsize<'a> = LinkedData<'a, usize>;
pub type LinkedIsize<'a> = LinkedData<'a, isize>;
pub type LinkedBool<'a> = LinkedData<'a, bool>;
