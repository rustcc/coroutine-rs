//! Coroutine implementation in Rust.
//!
//! ## Example
//!
//! ```rust
//! use coroutine::asymmetric::*;
//!
//! let mut coro = Coroutine::spawn(|coro, val| {
//!     println!("Inside {}", val);
//!     coro.yield_with(val + 1)
//! });
//!
//! println!("Resume1 {}", coro.resume(0).unwrap());
//! println!("Resume2 {}", coro.resume(2).unwrap());
//! ```
//!
//! This will prints
//!
//! ```plain
//! Inside 0
//! Resume1 1
//! Resume2 2
//! ```
#[macro_use]
extern crate log;
extern crate libc;
extern crate context;

use std::any::Any;
use std::error;
use std::fmt::{self, Display};
use std::panic;
use std::thread;

pub use options::Options;

pub mod asymmetric;
mod options;

/// Return type of resuming. Ok if resume successfully with the current state,
/// Err if resume failed with `Error`.
pub type Result<T> = ::std::result::Result<T, Error>;

/// Resume Error
pub enum Error {
    /// Coroutine is panicked
    Panicked,

    /// Coroutine is panicking, carry with the parameter of `panic!()`
    Panicking(Box<Any + Send>),
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            &Error::Panicked => write!(f, "Panicked"),
            &Error::Panicking(ref err) => {
                let msg = match err.downcast_ref::<&'static str>() {
                    Some(s) => *s,
                    None => {
                        match err.downcast_ref::<String>() {
                            Some(s) => &s[..],
                            None => "Box<Any>",
                        }
                    }
                };
                write!(f, "Panicking({})", msg)
            }
        }
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", error::Error::description(self))
    }
}

impl error::Error for Error {
    fn description(&self) -> &str {
        match self {
            &Error::Panicked => "Panicked",
            &Error::Panicking(..) => "Panicking(..)",
        }
    }
}

unsafe fn try<R, F: FnOnce() -> R>(f: F) -> thread::Result<R> {
    let mut f = Some(f);
    let f = &mut f as *mut Option<F> as usize;

    panic::catch_unwind(move || (*(f as *mut Option<F>)).take().unwrap()())
}
