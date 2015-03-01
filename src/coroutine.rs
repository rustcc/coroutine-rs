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
use std::ops::Deref;
use std::cell::RefCell;
use std::rc::Rc;

use context::Context;
use stack::{StackPool, Stack};

#[derive(Debug, Copy)]
pub enum State {
    Suspended,
    Running,
    Finished,
}

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
    parent: Option<Rc<RefCell<Coroutine>>>,

    /// State
    state: State
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

    COROUTINE_ENVIRONMENT.with(|env| {
        let mut env = env.borrow_mut();
        env.current_running.borrow_mut().state = State::Finished;
    });

    loop {
        Coroutine::sched();
    }
}

impl Coroutine {
    pub fn empty() -> Rc<RefCell<Coroutine>> {
        Rc::new(RefCell::new(Coroutine {
            current_stack_segment: None,
            saved_context: Context::empty(),
            parent: None,
            state: State::Running,
        }))
    }

    pub fn spawn_opts<F>(f: F, opts: Options) -> Rc<RefCell<Coroutine>>
            where F: FnOnce() + Send + 'static {
        let stack = RefCell::new(unsafe { Stack::dummy_stack() });
        COROUTINE_ENVIRONMENT.with(|env| {
            *stack.borrow_mut() = env.borrow_mut().stack_pool.take_stack(opts.stack_size);
        });

        let mut stack = stack.into_inner();
        let coro = Coroutine::empty();

        let ctx = Context::new(coroutine_initialize,
                               0,
                               f,
                               &mut stack);
        coro.borrow_mut().saved_context = ctx;
        coro.borrow_mut().current_stack_segment = Some(stack);

        Coroutine::resume(&coro);
        coro
    }

    /// Spawn a coroutine with default options
    pub fn spawn<F>(f: F) -> Rc<RefCell<Coroutine>>
            where F: FnOnce() + Send + 'static {
        Coroutine::spawn_opts(f, Default::default())
    }

    pub fn resume(coro: &Rc<RefCell<Coroutine>>) {
        match coro.borrow().state {
            State::Finished => return,
            _ => {}
        }

        COROUTINE_ENVIRONMENT.with(|env| {
            let current_running = env.borrow().current_running.clone();

            let to_coro: &mut Coroutine = unsafe { transmute(coro.borrow().deref()) };
            let from_coro: &mut Coroutine = unsafe { transmute(current_running.borrow().deref()) };

            to_coro.parent = Some(current_running.clone());
            env.borrow_mut().current_running = coro.clone();

            from_coro.state = State::Suspended;
            to_coro.state = State::Running;

            Context::swap(&mut from_coro.saved_context, &to_coro.saved_context);
        })
    }

    pub fn sched() {
        COROUTINE_ENVIRONMENT.with(|env| {
            let from_coro: &mut Coroutine = unsafe { transmute(env.borrow().current_running.deref()) };
            let to_coro: &mut Coroutine = match from_coro.parent {
                Some(ref parent) => unsafe {
                    env.borrow_mut().current_running = parent.clone();
                    transmute(parent.borrow().deref())
                },
                None => return,
            };

            match from_coro.state {
                State::Finished => {},
                _ => from_coro.state = State::Suspended,
            }
            to_coro.state = State::Running;

            Context::swap(&mut from_coro.saved_context, &to_coro.saved_context);
        });
    }

    pub fn current() -> Rc<RefCell<Coroutine>> {
        let opt = RefCell::new(None);
        COROUTINE_ENVIRONMENT.with(|env| {
            *opt.borrow_mut() = Some(env.borrow().current_running.clone());
        });
        opt.into_inner().unwrap()
    }

    pub fn state(&self) -> State {
        self.state
    }
}

thread_local!(static COROUTINE_ENVIRONMENT: RefCell<Environment> = RefCell::new(Environment::new()));

/// Coroutine managing environment
#[allow(raw_pointer_derive)]
#[derive(Debug)]
struct Environment {
    stack_pool: StackPool,
    current_running: Rc<RefCell<Coroutine>>,
    main_coroutine: Rc<RefCell<Coroutine>>,
}

impl Environment {
    /// Initialize a new environment
    fn new() -> Environment {
        let main_coro = Coroutine::empty();
        Environment {
            stack_pool: StackPool::new(),
            current_running: main_coro.clone(),
            main_coroutine: main_coro,
        }
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
        let coro = Coroutine::spawn(move|| {
            tx.send(1).unwrap();

            Coroutine::sched();

            tx.send(2).unwrap();
        });

        assert_eq!(rx.recv().unwrap(), 1);
        assert!(rx.try_recv().is_err());

        Coroutine::resume(&coro);

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
        let coro = Coroutine::spawn(move|| {});

        // It is already finished, but we try to resume it
        // Idealy it would come back immediately
        Coroutine::resume(&coro);

        // Again?
        Coroutine::resume(&coro);
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

            let coro = Coroutine::spawn(move|| {
                for _ in 0..MAX_NUMBER {
                    tx.send(1).unwrap();
                    Coroutine::sched();
                }
            });

            let result = rx.iter().fold(0, |a, b| {
                Coroutine::resume(&coro);
                a + b
            });
            assert_eq!(result, MAX_NUMBER);
        });
    }
}
