/// Like mem::epoch::AtomicPtr, but provides an ll/sc based api on x86, powerpc, arm, aarch64

use std::mem;
use std::marker::PhantomData;

use std::sync::atomic::{Ordering, AtomicUsize};
use std::sync::atomic::Ordering::*;

#[inline(awlays)]
unsafe fn load_exc(ptr: *const u64, ord: Ordering) {
    let rval: u64;
    match ord {
        Relaxed => {
            asm!("ldxr %1, [%0]"
                : "=r" (rval)
                : "r" (ptr)
                : "volatile")
        }

        Acquire | SeqCst => {
            asm!("ldaxr %1, [%0]"
                : "=r" (rval)
                : "r" (ptr)
                : "memory"
                : "volatile")
        }

        Release | AcqRel => panic!("Invalid load ordering"),
    }
    rval
}

#[inline(always)]
unsafe fn load_from(ptr: *const u64, ord: Ordering) -> u64 {
    let ptr: *const AtomicUsize = mem::transmute(ptr);
    (&*ptr).load(ord) as u64
}

#[inline(always)]
unsafe fn store_to(ptr: *const u64, n: u64, ord: Ordering) {
    let ptr: *const AtomicUsize = mem::transmute(ptr);
    (&*ptr).store(n as usize, ord)
}

#[inline(always)]
unsafe fn exchange_to(ptr: *const u64, n: u64, ord: Ordering) -> u64 {
    let ptr: *const AtomicUsize = mem::transmute(ptr);
    (&*ptr).swap(n as usize, ord) as u64
}


#[inline(always)]
unsafe fn store_exc(ptr: *const u64, val: u64, ord: Ordering,
                    rord: Ordering, reload: bool)
                    -> (bool, u64) {
    let succ: bool;
    match ord {
        Relaxed => {
            asm!("stxr %0 %1 [%2]"
                 : "=r" (succ)
                 : "r" (val), "r" (ptr)
                 : "memory"
                 : "volatile")
        }
        Release | SeqCst => {
            asm!("stlxr %0 %1 [%2]"
                 : "=r" (succ)
                 : "r" (val), "r" (ptr)
                 : "memory"
                 : "volatile")
        }
        Acquire | AcqRel => panic("Invalid Store Ordering"),
    }
    if succ {
        (true, 0)
    }
    else {
        (false, if reload { load_exc(ptr, rord) } else { mem::uninitialized() })
    }
}

pub trait IsU64 {
    fn from_u64(val: u64) -> Self;
    fn to_u64(&self) -> u64;
}

impl IsU64 for usize {
    fn from_u64(val: u64) -> usize {
        val as usize
    }

    fn to_u64(&self) -> u64 {
        *self as u64
    }
}

impl IsU64 for isize {
    fn from_u64(val: u64) -> isize {
        val as isize
    }

    fn to_u64(&self) -> u64 {
        *self as u64
    }
}

impl<T> IsU64 for *mut T {

    fn from_u64(val: u64) -> *mut T {
        val as *mut T
    }

    fn to_u64(&self) -> u64 {
        *self as u64
    }
}

impl IsU64 for bool {

    fn from_u64(val: u64) -> bool {
        val == 0
    }

    fn to_u64(&self) -> u64 {
        *self as u64
    }
}

pub struct ExclusiveData<T: IsU64> {
    data: u64,
    marker: PhantomData<T>,
}

pub struct LinkedData<'a, T: 'a + IsU64> {
    data: u64,
    ptr: *const u64,
    ord: Ordering,
    marker: PhantomData<'a, T>,
}

impl<T: IsU64> ExclusiveData<T> {

    pub fn new(val: T) -> ExclusiveData<T> {
        ExclusiveData {
            data: val.to_u64(),
            marker: PhantomData,
        }
    }

    /// Loads the value from the pointer with the given ordering
    pub fn load(&self, ord: Ordering) -> T {
        unsafe { T::from_u64(load_from(&self.data, ord)) }
    }

    /// Stores directly to the pointer without updating the counter
    ///
    /// This function can still leave one vulnerable to the ABA problem,
    /// But is useful when only used to store to say a null value.
    /// Be careful when using, this must always cause a store_conditional to fail
    pub fn store_direct(&self, val: T, ord: Ordering) {
        unsafe { store_to(&self.data, val.to_u64(), ord) };
    }

    /// Stores directly to the pointer without updating the counter
    ///
    /// This function can still leave one vulnerable to the ABA problem,
    /// But is useful when only used to store to say a null value.
    /// Be careful when using, this must always cause a store_conditional to fail
    pub fn exchange_direct(&self, val: T, ord: Ordering) -> T {
        unsafe { T::from_u64(exchange_to(&self.data, val.to_u64(), ord)) }
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
                data: load_from(&self.data, ord),
                ptr: &self.data,
                ord: ord,
                marker: PhantomData,
            }
        }
    }
}

impl<'a, T: IsU64> LinkedData<'a, T> {

    pub fn get(&self) -> T {
        T::from_u64(self.data)
    }

    /// Performs a conditional store on the pointer, conditional on no modifications occurring
    ///
    /// If the pointer is modified by a different store_conditional in between the load_linked
    /// and store_conditional, this will always fail. This is stronger the cas
    /// since cas can succedd when modifications have occured as long as the end
    /// result is the same. However, this will always fail in a scenario where cas would fail.
    pub fn store_conditional(self, val: T, _: Ordering) -> Option<LinkedData<'a, T>> {
        unsafe {
            let (succ, res) = cas_tagged(self.ptr, self.data, val.to_u64());
            match succ {
                true => None,
                false => Some(LinkedData {
                    data: res,
                    ptr: self.ptr,
                    marker: PhantomData,
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
        unsafe { cas_tagged(self.ptr, self.data, val.to_u64()).0 }
    }
}

unsafe impl<T: IsU64> Send for ExclusiveData<T> {}
unsafe impl<T: IsU64> Sync for ExclusiveData<T> {}

pub type ExclusivePtr<T> = ExclusiveData<*mut T>;
pub type ExclusiveUsize = ExclusiveData<usize>;
pub type ExclusiveIsize = ExclusiveData<isize>;

// This could be more efficient, by doing normal cas and packing
// as u64. BUT! That's code bloat for the time being
pub type ExclusiveBool = ExclusiveData<bool>;

pub type LinkedPtr<'a, T> = LinkedData<'a, *mut T>;
pub type LinkedUsize<'a> = LinkedData<'a, usize>;
pub type LinkedIsize<'a> = LinkedData<'a, isize>;
pub type LinkedBool<'a> = LinkedData<'a, bool>;
