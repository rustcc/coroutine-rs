
pub use self::inner_impl::{Coroutine, Handle};

#[cfg(feature = "enable-clonable-handle")]
pub use self::clonable as inner_impl;
#[cfg(not(feature = "enable-clonable-handle"))]
pub use self::unique as inner_impl;

#[cfg(feature = "enable-clonable-handle")]
pub mod clonable;
#[cfg(not(feature = "enable-clonable-handle"))]
pub mod unique;

pub mod asymmetric;
pub mod raw;
