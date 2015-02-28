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
use std::mem::{transmute, transmute_copy};
use std::rt::unwind::try;
use std::boxed::BoxAny;
use std::ops::Deref;
use std::ptr;
use std::cell::RefCell;

use context::Context;
use stack::{StackPool, Stack};

/// A coroutine is nothing more than a (register context, stack) pair.
#[allow(raw_pointer_derive)]
#[derive(Debug)]
pub struct Coroutine {
    /// The segment of stack on which the task is currently running or
    /// if the task is blocked, on which the task will resume
    /// execution.
    current_stack_segment: Option<Stack>,

    /// Always valid if the task is alive and not running.
    saved_context: Context,

    /// Parent coroutine, may always be valid.
    parent: *mut Coroutine,
}

/// Destroy coroutine and try to reuse std::stack segment.
impl Drop for Coroutine {
    fn drop(&mut self) {
        match self.current_stack_segment.take() {
            Some(stack) => {
                COROUTINE_ENVIRONMENT.with(|env| {
                    env.borrow_mut().stack_pool.give_stack(stack);
                });
            },
            None => {}
        }
    }
}

/// Coroutine spawn options
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

/// Initialization function for make context
extern "C" fn coroutine_initialize(_: usize, f: *mut ()) -> ! {
    let func: Box<Thunk> = unsafe { transmute(f) };

    if let Err(cause) = unsafe { try(move|| func.invoke(())) } {
        error!("Panicked inside: {:?}", cause.downcast::<&str>());
    }

    loop {
        Coroutine::sched();
    }
}

impl Coroutine {
    pub fn empty() -> Box<Coroutine> {
        Box::new(Coroutine {
            current_stack_segment: None,
            saved_context: Context::empty(),
            parent: ptr::null_mut(),
        })
    }

    pub fn spawn_opts<F>(f: F, opts: Options) -> Box<Coroutine>
            where F: FnOnce() + Send + 'static {
        let stack = RefCell::new(unsafe { Stack::dummy_stack() });
        COROUTINE_ENVIRONMENT.with(|env| {
            *stack.borrow_mut() = env.borrow_mut().stack_pool.take_stack(opts.stack_size);
        });

        let mut stack = stack.into_inner();
        let mut coro = Coroutine::empty();

        let ctx = Context::new(coroutine_initialize,
                               0,
                               f,
                               &mut stack);
        coro.saved_context = ctx;
        coro.current_stack_segment = Some(stack);

        coro.resume();
        coro
    }

    /// Spawn a coroutine with default options
    pub fn spawn<F>(f: F) -> Box<Coroutine>
            where F: FnOnce() + Send + 'static {
        Coroutine::spawn_opts(f, Default::default())
    }

    pub fn resume(&mut self) {
        COROUTINE_ENVIRONMENT.with(|env| {
            self.parent = env.borrow().current_running;
            env.borrow_mut().current_running = unsafe { transmute_copy(&self) };

            let mut parent: &mut Coroutine = unsafe { transmute(self.parent) };
            Context::swap(&mut parent.saved_context, &self.saved_context);
        })
    }

    pub fn sched() {
        COROUTINE_ENVIRONMENT.with(|env| {
            let mut current_running: &mut Coroutine = unsafe {
                transmute(env.borrow().current_running)
            };
            env.borrow_mut().current_running = current_running.parent;
            let parent: &mut Coroutine = unsafe {
                transmute(env.borrow().current_running)
            };
            Context::swap(&mut current_running.saved_context, &parent.saved_context);
        });
    }
}

thread_local!(static COROUTINE_ENVIRONMENT: RefCell<Environment> = RefCell::new(Environment::new()));

/// Coroutine managing environment
#[allow(raw_pointer_derive)]
#[derive(Debug)]
struct Environment {
    stack_pool: StackPool,
    current_running: *mut Coroutine,
    main_coroutine: Box<Coroutine>,
}

impl Environment {
    /// Initialize a new environment
    fn new() -> Environment {
        let mut env = Environment {
            stack_pool: StackPool::new(),
            current_running: ptr::null_mut(),
            main_coroutine: Coroutine::empty(),
        };

        env.current_running = unsafe { transmute(env.main_coroutine.deref()) };
        env
    }
}

#[cfg(test)]
mod test {
    use std::sync::mpsc::channel;

    use test::Bencher;

    use coroutine::Coroutine;

    #[test]
    fn test_coroutine_basic() {
        let (tx, rx) = channel();
        Coroutine::spawn(move|| {
            tx.send(1).unwrap();
        });

        assert_eq!(rx.recv().unwrap(), 1);
    }

    #[test]
    fn test_coroutine_yield() {
        let (tx, rx) = channel();
        let mut coro = Coroutine::spawn(move|| {
            tx.send(1).unwrap();

            Coroutine::sched();

            tx.send(2).unwrap();
        });

        assert_eq!(rx.recv().unwrap(), 1);
        assert!(rx.try_recv().is_err());

        coro.resume();

        assert_eq!(rx.recv().unwrap(), 2);
    }

    #[test]
    fn test_coroutine_spawn_inside() {
        let (tx, rx) = channel();
        Coroutine::spawn(move|| {
            tx.send(1).unwrap();

            Coroutine::spawn(move|| {
                tx.send(2).unwrap();
            });
        });

        assert_eq!(rx.recv().unwrap(), 1);
        assert_eq!(rx.recv().unwrap(), 2);
    }

    #[test]
    #[should_fail]
    fn test_coroutine_panic() {
        Coroutine::spawn(move|| {
            panic!("Panic inside a coroutine!!");
        });

        unreachable!();
    }

    #[test]
    #[should_fail]
    fn test_coroutine_child_panic() {
        Coroutine::spawn(move|| {

            Coroutine::spawn(move|| {
                panic!("Panic inside a coroutine's child!!");
            });

        });

        unreachable!();
    }

    #[test]
    fn test_coroutine_resume_after_finished() {
        let mut coro = Coroutine::spawn(move|| {});

        // It is already finished, but we try to resume it
        // Idealy it would come back immediately
        coro.resume();

        // Again?
        coro.resume();
    }

    #[bench]
    fn bench_coroutine_spawning_with_recycle(b: &mut Bencher) {
        b.iter(|| {
            Coroutine::spawn(move|| {});
        });
    }

    #[bench]
    fn bench_coroutine_counting(b: &mut Bencher) {
        b.iter(|| {
            const MAX_NUMBER: usize = 100;
            let (tx, rx) = channel();

            let mut coro = Coroutine::spawn(move|| {
                for _ in 0..MAX_NUMBER {
                    tx.send(1).unwrap();
                    Coroutine::sched();
                }
            });

            let result = rx.iter().fold(0, |a, b| {
                coro.resume();
                a + b
            });
            assert_eq!(result, MAX_NUMBER);
        });
    }
}
