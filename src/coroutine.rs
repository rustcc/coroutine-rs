// Copyright 2013 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

// Coroutines represent nothing more than a context and a stack
// segment.

use std::default::Default;
use std::rt::util::min_stack;
use std::thunk::Thunk;
use std::mem::transmute;
use std::rt::unwind::try;
use std::boxed::BoxAny;

use context::Context;
use stack::{StackPool, Stack};

/// A coroutine is nothing more than a (register context, stack) pair.
#[derive(Debug)]
pub struct Coroutine {
    /// The segment of stack on which the task is currently running or
    /// if the task is blocked, on which the task will resume
    /// execution.
    current_stack_segment: Stack,

    /// Always valid if the task is alive and not running.
    saved_context: Context,

    parent_context: Context,
}

#[derive(Debug)]
pub struct Options {
    stack_size: usize,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            stack_size: min_stack(),
        }
    }
}

extern "C" fn coroutine_initialize(arg: usize, f: *mut ()) -> ! {
    let func: Box<Thunk<&mut Box<Coroutine>, _>> = unsafe { transmute(f) };
    let coro: &mut Box<Coroutine> = unsafe { transmute(arg) };

    error!("Before invoke {:?}", coro);

    let coro_mov = unsafe { transmute(arg) };

    if let Err(cause) = unsafe { try(move|| func.invoke(coro_mov)) } {
        error!("Panicked inside: {:?}", cause.downcast::<&str>());
    }

    loop {
        error!("After invoke {:?}", coro);
        coro.yield_now();
    }

    unreachable!();
}

impl Coroutine {
    pub fn empty() -> Box<Coroutine> {
        Box::new(Coroutine {
            current_stack_segment: unsafe { Stack::dummy_stack() },
            saved_context: Context::empty(),
            parent_context: Context::empty(),
        })
    }

    pub fn spawn<F>(f: F, opts: Options, stack_pool: &mut StackPool) -> Box<Coroutine>
            where F: FnOnce(&mut Box<Coroutine>) + Send + 'static {
        let stack = stack_pool.take_stack(opts.stack_size);

        let mut coro = Box::new(Coroutine {
            current_stack_segment: stack,
            saved_context: Context::empty(),
            parent_context: Context::empty(),
        });

        let ctx = Context::new(coroutine_initialize,
                               unsafe { transmute(&coro) },
                               f,
                               &mut coro.current_stack_segment);
        coro.saved_context = ctx;
        coro.resume();
        coro
    }

    pub fn resume(&mut self) {
        Context::swap(&mut self.parent_context, &self.saved_context);
    }

    pub fn yield_now(&mut self) {
        Context::swap(&mut self.saved_context, &self.parent_context);
    }

    /// Destroy coroutine and try to reuse std::stack segment.
    pub fn recycle(self, stack_pool: &mut StackPool) {
        let Coroutine { current_stack_segment, .. } = self;
        stack_pool.give_stack(current_stack_segment);
    }
}

#[cfg(test)]
mod test {
    use std::sync::mpsc::channel;
    use std::default::Default;
    use std::mem::transmute_copy;

    use coroutine::Coroutine;
    use stack::StackPool;

    #[test]
    fn test_coroutine_basic() {
        let mut stack_pool = StackPool::new();

        let (tx, rx) = channel();
        let coro = Coroutine::spawn(move|_| {
            tx.send(1).unwrap();
        }, Default::default(), &mut stack_pool);

        coro.recycle(&mut stack_pool);
        assert_eq!(rx.recv().unwrap(), 1);
    }

    #[test]
    fn test_coroutine_yield() {
        let mut stack_pool = StackPool::new();

        let (tx, rx) = channel();
        let mut coro = Coroutine::spawn(move|coro| {
            tx.send(1).unwrap();

            let addr: usize = unsafe { transmute_copy(coro) };
            error!("Before {:#x}", addr);
            let addr: usize = unsafe { transmute_copy(coro) };
            error!("Before {:#x}", addr);

            error!("Before yield {:?}", coro);
            coro.yield_now();
            let addr: usize = unsafe { transmute_copy(coro) };
            error!("After {:#x}", addr);
            error!("After yield {:?}", coro);

            tx.send(2).unwrap();

            error!("Sended {:?}\n", coro);
        }, Default::default(), &mut stack_pool);

        assert_eq!(rx.recv().unwrap(), 1);
        assert!(rx.try_recv().is_err());

        error!("Before second resume {:?}", coro);

        coro.resume();

        assert_eq!(rx.recv().unwrap(), 2);

        coro.recycle(&mut stack_pool);
    }
}
