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

#![feature(box_syntax, std_misc, libc, asm, core, alloc, test, unboxed_closures, page_size)]
#![feature(rustc_private)]

#[macro_use] extern crate log;
extern crate libc;
extern crate test;
extern crate deque;
extern crate mmap;

pub use coroutine::Builder;
pub use coroutine::{Coroutine, spawn, sched, current};

pub mod context;
pub mod coroutine;
pub mod stack;
mod thunk; // use self-maintained thunk, because std::thunk is temporary. May be replaced by FnBox in the future.
mod sys;
