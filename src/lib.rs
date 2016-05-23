#![feature(asm)]

#[cfg(target_arch = "x86_64")]
mod exclusive_target {
    mod x86_64;
    //mod x86;
    pub use self::x86_64::{ExclusivePtr, ExclusiveUsize, ExclusiveIsize, ExclusiveBool};
    pub use self::x86_64::{LinkedPtr, LinkedUsize, LinkedIsize, LinkedBool};
    pub const IS_LOCK_FREE: bool = true;
}

#[cfg(target_arch = "x86")]
mod exclusive_target {
    mod x86;
    //mod x86;
    pub use self::x86::{ExclusivePtr, ExclusiveUsize, ExclusiveIsize, ExclusiveBool};
    pub use self::x86::{LinkedPtr, LinkedUsize, LinkedIsize, LinkedBool};
    pub const IS_LOCK_FREE: bool = true;
}

#[cfg(target_arch = "aarch64")]
mod exclusive_target {
    mod aarch64;
    pub use self::aarch64::{ExclusivePtr, ExclusiveUsize, ExclusiveIsize, ExclusiveBool};
    pub use self::aarch64::{LinkedPtr, LinkedUsize, LinkedIsize, LinkedBool};
    pub const IS_LOCK_FREE: bool = true;
}

#[cfg(not(any(target_arch = "x86_64",
              target_arch = "x86",
              target_arch = "aarch64",
              target_arch = "arm",
              target_arch = "powerpc",
              target_arch = "powerpc64")))]
mod exclusive_target {
    pub use super::generic::{ExclusivePtr, ExclusiveUsize, ExclusiveIsize, ExclusiveBool};
    pub use super::generic::{LinkedPtr, LinkedUsize, LinkedIsize, LinkedBool};
    pub const IS_LOCK_FREE: bool = false;
}

//always build the generic one
#[allow(dead_code)]
mod generic;


pub use self::exclusive_target::{ExclusivePtr, ExclusiveUsize, ExclusiveIsize, ExclusiveBool};
pub use self::exclusive_target::{LinkedPtr, LinkedUsize, LinkedIsize, LinkedBool};

#[inline(always)]
pub fn is_lock_free() -> bool {
    self::exclusive_target::IS_LOCK_FREE
}

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
        let num_run: usize = 10000;
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
