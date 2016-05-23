/// Like mem::epoch::AtomicPtr, but provides an ll/sc based api

use std::mem;
use std::marker::PhantomData;
use std::sync::atomic::{Ordering, AtomicUsize};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Mutex;

#[repr(C)]
struct Llsc {
    val: AtomicUsize,
    counter: AtomicUsize,
    m: Mutex<()>, // SLOW!!!!
}

impl Llsc {

    pub fn get_vals(&self, ord: Ordering) -> (usize, usize) {
        (self.val.load(ord), self.counter.load(Relaxed))
    }

    pub fn set_val(&self, val: usize) {
        let _guard = self.m.lock().unwrap();
        self.val.store(val, Relaxed);
    }

    pub fn xchg_val(&self, val: usize) -> usize {
        let _guard = self.m.lock().unwrap();
        let rval = self.val.load(Relaxed);
        self.val.store(val, Relaxed);
        rval
    }

    pub fn cas(&self, oval: usize, ctr: usize, nval: usize, _: bool)
               -> (bool, (usize, usize)) {
        let _guard = self.m.lock().unwrap();
        let cval = self.val.load(Relaxed);
        let c_counter = self.counter.load(Relaxed);
        if cval == oval && c_counter == ctr {
            self.val.store(nval, Relaxed);
            self.counter.store(ctr.wrapping_add(1), Relaxed);
            (true, unsafe { mem::uninitialized() })
        }
        else {
            (false, (cval, c_counter))
        }
    }
}

pub trait IsUsize {
    fn from_usize(val: usize) -> Self;
    fn to_usize(&self) -> usize;
}

impl IsUsize for usize {
    fn from_usize(val: usize) -> usize {
        val as usize
    }

    fn to_usize(&self) -> usize {
        *self as usize
    }
}

impl IsUsize for isize {
    fn from_usize(val: usize) -> isize {
        val as isize
    }

    fn to_usize(&self) -> usize {
        *self as usize
    }
}

impl<T> IsUsize for *mut T {

    fn from_usize(val: usize) -> *mut T {
        val as *mut T
    }

    fn to_usize(&self) -> usize {
        *self as usize
    }
}

impl IsUsize for bool {

    fn from_usize(val: usize) -> bool {
        val == 0
    }

    fn to_usize(&self) -> usize {
        *self as usize
    }
}

pub struct ExclusiveData<T: IsUsize> {
    data: Llsc,
    marker: PhantomData<T>,
}

pub struct LinkedData<'a, T: 'a + IsUsize> {
    data: (usize, usize),
    ex_ptr: &'a Llsc,
    marker: PhantomData<T>,
}

impl<T: IsUsize> ExclusiveData<T> {

    pub fn new(val: T) -> ExclusiveData<T> {
        ExclusiveData {
            data: Llsc {
                val: AtomicUsize::new(val.to_usize()),
                counter: AtomicUsize::new(val.to_usize()),
                m: Mutex::new(()),
            },
            marker: PhantomData,
        }
    }

    /// Loads the value from the pointer with the given ordering
    pub fn load(&self, ord: Ordering) -> T {
        T::from_usize(self.data.get_vals(ord).0)
    }

    /// Stores directly to the pointer without updating the counter
    ///
    /// This function can still leave one vulnerable to the ABA problem,
    /// But is useful when only used to store to say a null value.
    /// Be careful when using, this must always cause a store_conditional to fail
    pub fn store_direct(&self, val: T, _: Ordering) {
        self.data.set_val(val.to_usize())
    }

    /// Stores directly to the pointer without updating the counter
    ///
    /// This function can still leave one vulnerable to the ABA problem,
    /// But is useful when only used to store to say a null value.
    /// Be careful when using, this must always cause a store_conditional to fail
    pub fn exchange_direct(&self, val: T, _: Ordering) -> T {
        T::from_usize(self.data.xchg_val(val.to_usize()))
    }

    /// Performs an exclusive load on the pointer
    ///
    /// If the pointer is modified by a different store_conditional in between the load_linked
    /// and store_conditional, this will always fail. This is stronger the cas
    /// since cas can succedd when modifications have occured as long as the end
    /// result is the same. However, this will always fail in a scenario where cas would fail.
    pub fn load_linked(&self, ord: Ordering) -> LinkedData<T> {
        LinkedData {
            data: self.data.get_vals(ord),
            ex_ptr: &self.data,
            marker: PhantomData,
        }
    }
}

impl<'a, T: IsUsize> LinkedData<'a, T> {

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
        let (succ, res) = self.ex_ptr.cas(self.data.0,
                                          self.data.1,
                                          val.to_usize(),
                                          true);
        match succ {
            true => None,
            false => Some(LinkedData {
                data: res,
                ex_ptr: self.ex_ptr,
                marker: PhantomData
            })
        }
    }

    /// Performs a conditional store on the pointer, conditional on no modifications occurring
    ///
    /// If the pointer is modified by a different store_conditional in between the load_linked
    /// and store_conditional, this will always fail. This is stronger the cas
    /// since cas can succedd when modifications have occured as long as the end
    /// result is the same. However, this will always fail in a scenario where cas would fail.
    pub fn try_store_conditional(self, val: T, _: Ordering) -> bool {
        self.ex_ptr.cas(self.data.0, self.data.1, val.to_usize(), false).0
    }
}

unsafe impl<T: IsUsize> Send for ExclusiveData<T> {}
unsafe impl<T: IsUsize> Sync for ExclusiveData<T> {}

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

#[cfg(test)]
mod test {
    extern crate crossbeam;
    use self::crossbeam::scope;
    use super::*;
    use std::ptr;
    use std::sync::atomic::Ordering::{Relaxed};
    #[test]
    fn test_cas () {
        let mut val: usize = 0;
        let eptr = ExclusivePtr::<usize>::new(ptr::null_mut());
        let ll = eptr.load_linked(Relaxed);
        assert_eq!(eptr.load(Relaxed), ptr::null_mut());
        assert_eq!(ll.try_store_conditional(&mut val, Relaxed), true);
        assert_eq!(eptr.load(Relaxed), &mut val as *mut usize);
    }

    #[test]
    fn test_cas_fail () {
        let mut val: usize = 0;
        let mut val2: usize = 0;
        let eptr = ExclusivePtr::<usize>::new(ptr::null_mut());
        let ll = eptr.load_linked(Relaxed);
        assert_eq!(eptr.load(Relaxed), ptr::null_mut());
        eptr.store_direct(&mut val2, Relaxed);
        assert_eq!(eptr.load(Relaxed), &mut val2 as *mut usize);
        assert_eq!(ll.store_conditional(&mut val, Relaxed).is_some(), true);
        assert_eq!(eptr.load(Relaxed), &mut val2 as *mut usize);
    }

    #[test]
    fn test_cas_fail_xchg () {
        let mut val: usize = 0;
        let mut val2: usize = 0;
        let eptr = ExclusivePtr::<usize>::new(ptr::null_mut());
        let ll = eptr.load_linked(Relaxed);
        assert_eq!(eptr.load(Relaxed), ptr::null_mut());
        assert_eq!(eptr.exchange_direct(&mut val2, Relaxed), ptr::null_mut());
        assert_eq!(eptr.load(Relaxed), &mut val2 as *mut usize);
        assert_eq!(ll.store_conditional(&mut val, Relaxed).is_some(), true);
        assert_eq!(eptr.load(Relaxed), &mut val2 as *mut usize);
    }

    #[test]
    fn test_mt_cas() {
        let num_run: usize = 100000;
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
