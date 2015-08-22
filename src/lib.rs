// Copyright 2013 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

// #![license = "MIT/ASL2"]
#![doc(html_logo_url = "http://www.rust-lang.org/logos/rust-logo-128x128-blk-v2.png",
       html_favicon_url = "http://www.rust-lang.org/favicon.ico")]

#![allow(unused_features)]
#![feature(std_misc, libc, asm, core, alloc, test, unboxed_closures, page_size, core_simd, rt)]
#![feature(rustc_private)]
#![feature(unique)]

#[macro_use] extern crate log;
extern crate libc;
extern crate test;
extern crate mmap;
#[cfg(feature = "enable-clonable-handle")]
extern crate spin;

use std::any::Any;
use std::error;
use std::fmt::{self, Display};

#[cfg(feature = "enable-clonable-handle")]
pub use coroutine_clonable as coroutine;

#[cfg(not(feature = "enable-clonable-handle"))]
pub use coroutine_unique as coroutine;

pub use builder::Builder;
pub use coroutine::{Coroutine, Handle};
pub use options::Options;

mod context;
#[cfg(feature = "enable-clonable-handle")]
pub mod coroutine_clonable;
#[cfg(not(feature = "enable-clonable-handle"))]
pub mod coroutine_unique;

pub mod builder;
pub mod options;
mod stack;
mod thunk; // use self-maintained thunk, because std::thunk is temporary. May be replaced by FnBox in the future.
mod sys;
mod environment;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod benchmarks;

/// Spawn a new Coroutine
///
/// Equavalent to `Coroutine::spawn`.
pub fn spawn<F>(f: F) -> Handle
    where F: FnOnce() + Send + 'static
{
    Builder::new().spawn(f)
}

/// Get the current Coroutine
///
/// Equavalent to `Coroutine::current`.
pub fn current() -> &'static Handle {
    Coroutine::current()
}

/// Resume a Coroutine
///
/// Equavalent to `Coroutine::resume`.
pub fn resume(coro: &Handle) -> Result {
    coro.resume()
}

/// Yield the current Coroutine with `Suspended` state
///
/// Equavalent to `Coroutine::sched`.
pub fn sched() {
    Coroutine::sched()
}

/// Yield the current Coroutine with `Blocked` state
///
/// Equavalent to `Coroutine::block`.
pub fn block() {
    Coroutine::block()
}

/// State of a Coroutine
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum State {
    /// Waiting its child to return
    Normal,

    /// Suspended. Can be waked up by `resume`
    Suspended,

    /// Blocked. Can be waked up by `resume`
    Blocked,

    /// Running
    Running,

    /// Finished
    Finished,

    /// Panic happened inside, cannot be resumed again
    Panicked,
}

/// Return type of resuming. Ok if resume successfully with the current state,
/// Err if resume failed with `Error`.
pub type Result = ::std::result::Result<State, Error>;

/// Resume Error
#[derive(Debug)]
pub enum Error {
    /// Coroutine is already finished
    Finished,

    /// Coroutine is waiting for its child to yield (state is Normal)
    Waiting,

    /// Coroutine is panicked
    Panicked,

    /// Coroutine is panicking, carry with the parameter of `panic!()`
    Panicking(Box<Any + Send>),
}

impl Error {
    pub fn description(&self) -> &str {
        match *self {
            Error::Finished => "Finished",
            Error::Waiting => "Waiting",
            Error::Panicked => "Panicked",
            Error::Panicking(ref err) => {
                match err.downcast_ref::<&'static str>() {
                    Some(s) => *s,
                    None => match err.downcast_ref::<String>() {
                        Some(s) => &s[..],
                        None => "Box<Any>",
                    }
                }
            }
        }
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.description())
    }
}

impl error::Error for Error {
    fn description(&self) -> &str {
        self.description()
    }

    fn cause(&self) -> Option<&error::Error> {
        Some(self)
    }
}
