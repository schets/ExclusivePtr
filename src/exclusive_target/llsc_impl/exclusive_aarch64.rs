/// Like mem::epoch::AtomicPtr, but provides an ll/sc based api on x86, powerpc, arm, aarch64

use std::mem;
use std::marker::PhantomData;

use std::sync::atomic::{Ordering, AtomicUsize};
use std::sync::atomic::Ordering::*;


#[cfg(target_arch = "aarch64")]
mod multi_arch {
    #[inline(awlays)]
    unsafe fn load_exc(ptr: *const usize, ord: Ordering, _: bool) {
        let rval: usize;
        match ord {
            Relaxed => {
                asm!("ldxr %1, [%0]"
                    : "=r" (rval)
                    : "r" (ptr)
                    : "volatile")
            },
            Acquire | SeqCst => {
                asm!("ldaxr %1, [%0]"
                    : "=r" (rval)
                    : "r" (ptr)
                    : "volatile")
            },
            Release | AcqRel => panic!("Invalid load ordering"),
        }
        rval
    }

    #[inline(always)]
    unsafe fn store_exc(ptr: *const usize, val: usize, ord: Ordering,
                        rord: Ordering, reload: bool) -> (bool, usize) {
        let succ: bool;
        match ord {
            Relaxed => {
                asm!("stxr %0 %1 [%2]"
                   : "=r" (succ)
                   : "r" (val), "r" (ptr)
                   : "memory"
                   : "volatile")
            },
            Release | SeqCst => {
                asm!("stlxr %0 %1 [%2]"
                   : "=r" (succ)
                   : "r" (val), "r" (ptr)
                   : "memory"
                   : "volatile")
            },
            Acquire | AcqRel => panic("Invalid Store Ordering"),
        }
        if succ {
            (true, mem::uninitialized() )
        }
        else {
            (false, if reload { load_exc(ptr, rord, false) }
                    else { mem::uninitialized() })
        }
    }
}

#[cfg(target_arch = "arm")]
mod multi_arch {

    // This may be able to eliminate a dmb sy in the mismatched seqcst case?

    #[inline(awlays)]
    unsafe fn load_exc(ptr: *const usize, ord: Ordering, rseqcst: bool) {
        let rval: usize;
        // This flag allows more efficient ll/sc loops when the ll/sc
        // flag reloads!
        if rseqcst && ord == SeqCst { asm!("dmb sy":::"memory":"volatile") }
        asm!("ldxr %1, [%0]"
             : "=r" (rval)
             : "r" (ptr)
             : "volatile");
        match ord {
            Relaxed => (),
            Acquire | SeqCst => asm!("dmb sy":::"memory":"volatile"),
            Release | AcqRel => panic!("Invalid load ordering"),
        }
        rval
    }

    #[inline(always)]
    unsafe fn store_exc(ptr: *const usize, val: usize, ord: Ordering,
                        rord: Ordering, reload: bool) -> (bool, usize) {
        let succ: bool;

        match ord {
            Relaxed => (),
            Release | SeqCst => asm!("dmb sy":::"memory":"volatile"),
            Acquire | AcqRel => panic("Invalid Store Ordering"),
        }
        asm!("strex %0 %1 [%2]"
             : "=r" (succ)
             : "r" (val), "r" (ptr)
             : "memory"
             : "volatile");
        if ord == SeqCst { asm!("dmb sy":::"memory":"volatile") }
        if succ {
            (true, mem::uninitialized() )
        }
        else {
            (false, if reload { load_exc(ptr, rord, ord != SeqCst) }
                    else { mem::uninitialized() })
        }
    }
}

#[cfg(target_arch = "powerpc")]
mod multi_arch {


    #[inline(awlays)]
    unsafe fn load_exc(ptr: *const usize, ord: Ordering, _: bool) {
        let rval: usize;
        // This flag allows more efficient ll/sc loops when the ll/sc
        // flag reloads!
        if ord == SeqCst { asm!("sync":::"memory":"volatile") }

        match ord {
            Relaxed => asm!("lwarx $0, 0, $1"
                            : "=r" (rval)
                            : "r" (ptr)
                            : "volatile"),
            Acquire | SeqCst => asm!("lwarx $0, 0, $1
                                      cmpw $1, $1
                                      bne- $+4
                                      isync"
                                      : "=r" (rval)
                                      : "r" (ptr)
                                      : "memory"
                                      : "volatile"),
            Release | AcqRel => panic!("Invalid load ordering"),
        }
        rval
    }

    #[inline(always)]
    unsafe fn store_exc(ptr: *const usize, val: usize, ord: Ordering,
                        rord: Ordering, reload: bool) -> (bool, usize) {
        let succ: usize;

        match ord {
            Relaxed => (),
            Release => asm!("lwsync":::"memory":"volatile"),
            SeqCst => asm!("sync":::"memory":"volatile"),
            Acquire | AcqRel => panic("Invalid Store Ordering"),
        }

        // The docs for powerpc condition register
        // and stwcx are absolute nonsense...
        // just copying from gcc and hoping that it works...
        asm!("stwcx. %1, 0, %2
              mfcr %0
              rlwinm %0, %0, 3, 1"
             : "=r" (succ)
             : "r" (val), "r" (ptr)
             : "memory"
             : "volatile");
        if succ == 0 {
            (true, mem::uninitialized() )
        }
        else {
            (false, if reload { load_exc(ptr, rord, false) }
                    else { mem::uninitialized() })
        }
    }
}

#[inline(always)]
unsafe fn load_from(ptr: *const usize, ord: Ordering) -> usize {
    let ptr: *const AtomicUsize = mem::transmute(ptr);
    (&*ptr).load(ord) as usize
}

#[inline(always)]
unsafe fn store_to(ptr: *const usize, n: usize, ord: Ordering) {
    let ptr: *const AtomicUsize = mem::transmute(ptr);
    (&*ptr).store(n as usize, ord)
}

#[inline(always)]
unsafe fn exchange_to(ptr: *const usize, n: usize, ord: Ordering) -> usize {
    let ptr: *const AtomicUsize = mem::transmute(ptr);
    (&*ptr).swap(n as usize, ord) as usize
}

#[inline(always)]
unsafe fn cas_to(ptr: *const usize, o: usize, n: usize, ord: Ordering) -> usize {
    let ptr: *const AtomicUsize = mem::transmute(ptr);
    (&*ptr).compare_and_swap(o as usize, n as usize, ord) as usize
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
    data: usize,
    marker: PhantomData<T>,
}

pub struct LinkedData<'a, T: 'a + Isusize> {
    data: usize,
    ptr: *const usize,
    ord: Ordering,
    marker: PhantomData<'a, T>,
}

impl<T: Isusize> ExclusiveData<T> {

    pub fn new(val: T) -> ExclusiveData<T> {
        ExclusiveData {
            data: val.to_usize(),
            marker: PhantomData,
        }
    }

    /// Loads the value from the pointer with the given ordering
    pub fn load(&self, ord: Ordering) -> T {
        unsafe { T::from_usize(load_from(&self.data, ord)) }
    }

    /// Stores directly to the pointer without updating the counter
    ///
    /// This function can still leave one vulnerable to the ABA problem,
    /// But is useful when only used to store to say a null value.
    /// Be careful when using, this must always cause a store_conditional to fail
    pub fn store_direct(&self, val: T, ord: Ordering) {
        unsafe { store_to(&self.data, val.to_usize(), ord) };
    }

    /// Swaps directly to the pointer without updating the counter
    ///
    /// This function can still leave one vulnerable to the ABA problem,
    /// But is useful when only used to store to say a null value.
    /// Be careful when using, this must always cause a store_conditional to fail
    pub fn swap_direct(&self, val: T, ord: Ordering) -> T {
        unsafe { T::from_usize(exchange_to(&self.data, val.to_usize(), ord)) }
    }

    /// Cas's directly to the pointer without updating the counter
    ///
    /// This function can still leave one vulnerable to the ABA problem,
    /// But is useful when only used to store to say a null value.
    /// Be careful when using, this must always cause a store_conditional to fail
    pub fn cas_direct(&self, old: T, val: T, ord: Ordering) -> T {
        unsafe { T::from_usize(cas_to(&self.data, old.to_usize(),
                                    val.to_usize(), ord)) }
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

impl<'a, T: Isusize> LinkedData<'a, T> {

    pub fn get(&self) -> T {
        T::from_usize(self.data)
    }

    /// Performs a conditional store on the pointer, conditional on no modifications occurring
    ///
    /// If the pointer is modified by a different store_conditional in between the load_linked
    /// and store_conditional, this will always fail. This is stronger the cas
    /// since cas can succedd when modifications have occured as long as the end
    /// result is the same. However, this will always fail in a scenario where cas would fail.
    pub fn store_conditional(self, val: T, ord: Ordering) -> Option<LinkedData<'a, T>> {
        unsafe {
            let (succ, res) = store_exc(self.ptr, val.to_usize(), ord,
                                        self.ord, true);
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
        unsafe { store_exc(self.ptr, val.to_usize(), ord, self.ord, false).0 }
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
