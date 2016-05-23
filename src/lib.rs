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
    mod generic;
    pub use self::x86_64::{ExclusivePtr, ExclusiveUsize, ExclusiveIsize, ExclusiveBool};
    pub use self::x86_64::{LinkedPtr, LinkedUsize, LinkedIsize, LinkedBool};
    pub const IS_LOCK_FREE: bool = false;
}

//always build the generic one


pub use self::exclusive_target::{ExclusivePtr, ExclusiveUsize, ExclusiveIsize, ExclusiveBool};
pub use self::exclusive_target::{LinkedPtr, LinkedUsize, LinkedIsize, LinkedBool};

#[inline(always)]
pub fn is_lock_free() -> bool {
    self::exclusive_target::IS_LOCK_FREE
}
