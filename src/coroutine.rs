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
use std::ops::{Deref, DerefMut};
use std::ptr;

use context::Context;
use stack::{StackPool, Stack};

/// A coroutine is nothing more than a (register context, stack) pair.
#[allow(raw_pointer_derive)]
#[derive(Debug)]
pub struct Coroutine {
    /// The segment of stack on which the task is currently running or
    /// if the task is blocked, on which the task will resume
    /// execution.
    current_stack_segment: Stack,

    /// Always valid if the task is alive and not running.
    saved_context: Context,

    /// Parent coroutine, may always be valid.
    parent: *mut Coroutine,
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
extern "C" fn coroutine_initialize(arg: usize, f: *mut ()) -> ! {
    let func: Box<Thunk<&mut Environment, _>> = unsafe { transmute(f) };
    let environment: &mut Environment = unsafe { transmute(arg) };

    let env_mov = unsafe { transmute(arg) };

    if let Err(cause) = unsafe { try(move|| func.invoke(env_mov)) } {
        error!("Panicked inside: {:?}", cause.downcast::<&str>());
    }

    loop {
        // coro.yield_now();
        environment.yield_now();
    }
}

impl Coroutine {
    pub fn empty() -> Box<Coroutine> {
        Box::new(Coroutine {
            current_stack_segment: unsafe { Stack::dummy_stack() },
            saved_context: Context::empty(),
            parent: ptr::null_mut(),
        })
    }

    /// Destroy coroutine and try to reuse std::stack segment.
    pub fn recycle(self, stack_pool: &mut StackPool) {
        let Coroutine { current_stack_segment, .. } = self;
        stack_pool.give_stack(current_stack_segment);
    }
}

/// Coroutine managing environment
#[allow(raw_pointer_derive)]
#[derive(Debug)]
pub struct Environment {
    stack_pool: StackPool,
    current_running: *mut Coroutine,
    main_coroutine: Box<Coroutine>,
}

impl Environment {
    /// Initialize a new environment
    pub fn new() -> Box<Environment> {
        let mut env = box Environment {
            stack_pool: StackPool::new(),
            current_running: ptr::null_mut(),
            main_coroutine: Coroutine::empty(),
        };

        env.current_running = unsafe { transmute(env.main_coroutine.deref()) };
        env
    }

    /// Spawn a new coroutine with options
    pub fn spawn_opts<F>(&mut self, f: F, opts: Options) -> Box<Coroutine>
            where F: FnOnce(&mut Environment) + Send + 'static {

        let mut coro = box Coroutine {
            current_stack_segment: self.stack_pool.take_stack(opts.stack_size),
            saved_context: Context::empty(),
            parent: ptr::null_mut(),
        };

        let ctx = Context::new(coroutine_initialize,
                               unsafe { transmute_copy(&self) },
                               f,
                               &mut coro.current_stack_segment);
        coro.saved_context = ctx;

        self.resume(&mut coro);
        coro
    }

    /// Spawn a coroutine with default options
    pub fn spawn<F>(&mut self, f: F) -> Box<Coroutine>
            where F: FnOnce(&mut Environment) + Send + 'static {
        self.spawn_opts(f, Default::default())
    }

    /// Yield the current running coroutine
    pub fn yield_now(&mut self) {
        let mut current_running: &mut Coroutine = unsafe { transmute(self.current_running) };
        self.current_running = current_running.parent;
        let parent: &mut Coroutine = unsafe { transmute(self.current_running) };
        Context::swap(&mut current_running.saved_context, &parent.saved_context);
    }

    /// Suspend the current coroutine and resume the `coro` now
    pub fn resume(&mut self, coro: &mut Box<Coroutine>) {
        coro.parent = self.current_running;
        self.current_running = unsafe { transmute_copy(&coro.deref_mut()) };

        let mut parent: &mut Coroutine = unsafe { transmute(coro.parent) };
        Context::swap(&mut parent.saved_context, &coro.saved_context);
    }

    /// Destroy the coroutine
    pub fn recycle(&mut self, coro: Box<Coroutine>) {
        coro.recycle(&mut self.stack_pool)
    }
}

#[cfg(test)]
mod test {
    use std::sync::mpsc::channel;

    use test::Bencher;

    use coroutine::Environment;

    #[test]
    fn test_coroutine_basic() {
        let mut env = Environment::new();

        let (tx, rx) = channel();
        let coro = env.spawn(move|_| {
            tx.send(1).unwrap();
        });

        assert_eq!(rx.recv().unwrap(), 1);

        env.recycle(coro);
    }

    #[test]
    fn test_coroutine_yield() {
        let mut env = Environment::new();

        let (tx, rx) = channel();
        let mut coro = env.spawn(move|env| {
            tx.send(1).unwrap();

            env.yield_now();

            tx.send(2).unwrap();
        });

        assert_eq!(rx.recv().unwrap(), 1);
        assert!(rx.try_recv().is_err());

        env.resume(&mut coro);

        assert_eq!(rx.recv().unwrap(), 2);

        env.recycle(coro);
    }

    #[test]
    fn test_coroutine_spawn_inside() {
        let mut env = Environment::new();

        let (tx, rx) = channel();
        let coro = env.spawn(move|env| {
            tx.send(1).unwrap();

            let subcoro = env.spawn(move|_| {
                tx.send(2).unwrap();
            });

            env.recycle(subcoro);
        });

        assert_eq!(rx.recv().unwrap(), 1);
        assert_eq!(rx.recv().unwrap(), 2);

        env.recycle(coro);
    }

    #[test]
    #[should_fail]
    fn test_coroutine_panic() {
        let mut env = Environment::new();

        env.spawn(move|_| {
            panic!("Panic inside a coroutine!!");
        });

        unreachable!();
    }

    #[test]
    #[should_fail]
    fn test_coroutine_child_panic() {
        let mut env = Environment::new();

        env.spawn(move|env| {

            env.spawn(move|_| {
                panic!("Panic inside a coroutine's child!!");
            });

        });

        unreachable!();
    }

    #[test]
    fn test_coroutine_resume_after_finished() {
        let mut env = Environment::new();

        let mut coro = env.spawn(move|_| {});

        // It is already finished, but we try to resume it
        // Idealy it would come back immediately
        env.resume(&mut coro);

        // Again?
        env.resume(&mut coro);
    }

    #[bench]
    fn bench_coroutine_spawning_without_recycle(b: &mut Bencher) {
        b.iter(|| {
            Environment::new().spawn(move|_| {});
        });
    }

    #[bench]
    fn bench_coroutine_spawning_with_recycle(b: &mut Bencher) {
        let mut env = Environment::new();
        b.iter(|| {
            let coro = env.spawn(move|_| {});
            env.recycle(coro);
        });
    }

    #[bench]
    fn bench_coroutine_counting(b: &mut Bencher) {
        b.iter(|| {
            let mut env = Environment::new();

            const MAX_NUMBER: usize = 100;
            let (tx, rx) = channel();

            let mut coro = env.spawn(move|env| {
                for _ in 0..MAX_NUMBER {
                    tx.send(1).unwrap();
                    env.yield_now();
                }
            });

            let result = rx.iter().fold(0, |a, b| {
                env.resume(&mut coro);
                a + b
            });
            assert_eq!(result, MAX_NUMBER);
        });
    }
}
