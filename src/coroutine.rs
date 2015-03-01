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
use thunk::Thunk;
use std::mem::transmute;
use std::rt::unwind::try;
use std::any::Any;
use std::cell::UnsafeCell;
use std::rc::Rc;

use context::Context;
use stack::{StackPool, Stack};

#[derive(Debug, Copy)]
pub enum State {
    Suspended,
    Running,
    Finished,
    Panicked,
}

pub type ResumeResult<T> = Result<T, Box<Any + Send>>;

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

/// Handle of a Coroutine
#[derive(Clone)]
pub struct Handle(Rc<UnsafeCell<Coroutine>>);

impl Handle {
    pub fn resume(&self) -> ResumeResult<()> {
        Coroutine::resume(&self)
    }

    pub fn state(&self) -> State {
        Coroutine::state(&self)
    }
}

/// A coroutine is nothing more than a (register context, stack) pair.
#[allow(raw_pointer_derive)]
pub struct Coroutine {
    /// The segment of stack on which the task is currently running or
    /// if the task is blocked, on which the task will resume
    /// execution.
    current_stack_segment: Option<Stack>,

    /// Always valid if the task is alive and not running.
    saved_context: Context,

    /// Parent coroutine, may always be valid.
    parent: Option<Handle>,

    /// State
    state: State,
}

/// Destroy coroutine and try to reuse std::stack segment.
impl Drop for Coroutine {
    fn drop(&mut self) {
        match self.current_stack_segment.take() {
            Some(stack) => {
                COROUTINE_ENVIRONMENT.with(|env| {
                    let env: &mut Environment = unsafe { transmute(env.get()) };
                    env.stack_pool.give_stack(stack);
                });
            },
            None => {}
        }
    }
}

/// Initialization function for make context
extern "C" fn coroutine_initialize(_: usize, f: *mut ()) -> ! {
    let func: Box<Thunk> = unsafe { transmute(f) };

    let ret = unsafe { try(move|| func.invoke(())) };

    COROUTINE_ENVIRONMENT.with(move|env| {
        let env: &mut Environment = unsafe { transmute(env.get()) };
        let cur: &mut Coroutine = unsafe { transmute(env.current_running.0.get()) };

        match ret {
            Ok(..) => {
                env.running_state = None;
                cur.state = State::Finished;
            }
            Err(err) => {
                env.running_state = Some(err);
                cur.state = State::Panicked;
            }
        }
    });

    loop {
        Coroutine::sched();
    }
}

impl Coroutine {
    pub fn empty() -> Coroutine {
        Coroutine {
            current_stack_segment: None,
            saved_context: Context::empty(),
            parent: None,
            state: State::Running,
        }
    }
    pub fn handle(self) -> Handle {
        Handle(Rc::new(UnsafeCell::new(self)))
    }

    pub fn spawn_opts<F>(f: F, opts: Options) -> Handle where F: FnOnce() + Send + 'static {
        COROUTINE_ENVIRONMENT.with(|env| unsafe {
            let env: &mut Environment = transmute(env.get());
            
            let mut stack = env.stack_pool.take_stack(opts.stack_size);
            
            let ctx = Context::new(coroutine_initialize,
                                   0,
                                   f,
                                   &mut stack);
            let mut coro = Coroutine::empty();
            coro.saved_context = ctx;
            coro.current_stack_segment = Some(stack);
            coro.state = State::Suspended;
            coro
        }).handle()
    }

    /// Spawn a coroutine with default options
    pub fn spawn<F>(f: F) -> Handle where F: FnOnce() + Send + 'static {
        Coroutine::spawn_opts(f, Default::default())
    }

    pub fn resume(coro: &Handle) -> ResumeResult<()> {
        let to_coro: &mut Coroutine = unsafe { transmute(coro.0.get()) };
        match to_coro.state {
            State::Finished | State::Running => return Ok(()),
            State::Panicked => panic!("Tried to resume a panicked coroutine"),
            _ => {}
        }
        COROUTINE_ENVIRONMENT.with(|env| {
            let env: &mut Environment = unsafe { transmute(env.get()) };
            
            let current_running = env.current_running.clone();
            
            let from_coro: &mut Coroutine = unsafe { transmute(current_running.0.get()) };
            
            to_coro.parent = Some(current_running.clone());
            env.current_running = coro.clone();
            
            from_coro.state = State::Suspended;
            to_coro.state = State::Running;
            
            Context::swap(&mut from_coro.saved_context, &to_coro.saved_context);
            
            match env.running_state.take() {
                Some(err) => {
                    Err(err)
                },
                None => {
                    Ok(())
                }
            }
        })
    }

    pub fn sched() {
        COROUTINE_ENVIRONMENT.with(|env| {
            let env: &mut Environment = unsafe { transmute(env.get()) };

            let from_coro: &mut Coroutine = unsafe { transmute(env.current_running.0.get()) };
            let to_coro: &mut Coroutine = match from_coro.parent {
                Some(ref parent) => unsafe {
                    env.current_running = parent.clone();
                    transmute(parent.0.get())
                },
                None => return,
            };

            match from_coro.state {
                State::Finished | State::Panicked => {},
                _ => from_coro.state = State::Suspended,
            }
            to_coro.state = State::Running;

            Context::swap(&mut from_coro.saved_context, &to_coro.saved_context);
        });
    }

    pub fn current() -> Handle {
        COROUTINE_ENVIRONMENT.with(|env| unsafe {
            let env: &mut Environment = transmute(env.get());
            env.current_running.clone()
        })
    }

    pub fn state(coro: &Handle) -> State {
        let coro: &mut Coroutine = unsafe { transmute(coro.0.get()) };
        coro.state
    }
}

thread_local!(static COROUTINE_ENVIRONMENT: UnsafeCell<Environment> = UnsafeCell::new(Environment::new()));

/// Coroutine managing environment
#[allow(raw_pointer_derive)]
struct Environment {
    stack_pool: StackPool,
    current_running: Handle,

    running_state: Option<Box<Any + Send>>,
}

impl Environment {
    /// Initialize a new environment
    fn new() -> Environment {
        Environment {
            stack_pool: StackPool::new(),
            current_running: Coroutine::empty().handle(),

            running_state: None,
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
        let coro = Coroutine::spawn(move|| {
            tx.send(1).unwrap();
        });
        assert!(coro.resume().is_ok());

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

        assert!(coro.resume().is_ok());

        assert_eq!(rx.recv().unwrap(), 1);
        assert!(rx.try_recv().is_err());

        assert!(coro.resume().is_ok());

        assert_eq!(rx.recv().unwrap(), 2);
    }

    #[test]
    fn test_coroutine_spawn_inside() {
        let (tx, rx) = channel();
        let coro = Coroutine::spawn(move|| {
            tx.send(1).unwrap();

            let coro = Coroutine::spawn(move|| {
                tx.send(2).unwrap();
            });

            assert!(coro.resume().is_ok());
        });

        assert!(coro.resume().is_ok());

        assert_eq!(rx.recv().unwrap(), 1);
        assert_eq!(rx.recv().unwrap(), 2);
    }

    #[test]
    fn test_coroutine_panic() {
        let coro = Coroutine::spawn(move|| {
            panic!("Panic inside a coroutine!!");
        });
        assert!(coro.resume().is_err());
    }

    #[test]
    fn test_coroutine_child_panic() {
        let coro = Coroutine::spawn(move|| {

            let coro = Coroutine::spawn(move|| {
                panic!("Panic inside a coroutine's child!!");
            });

            assert!(coro.resume().is_err());
        });

        assert!(coro.resume().is_ok());
    }

    #[test]
    fn test_coroutine_resume_after_finished() {
        let coro = Coroutine::spawn(move|| {});

        // It is already finished, but we try to resume it
        // Idealy it would come back immediately
        assert!(Coroutine::resume(&coro).is_ok());

        // Again?
        assert!(Coroutine::resume(&coro).is_ok());
    }

    #[test]
    fn test_coroutine_resume_itself() {
        let coro = Coroutine::spawn(move|| {
            // Resume itself
            Coroutine::current().resume().unwrap();
        });

        assert!(coro.resume().is_ok());
    }

    #[test]
    fn test_coroutine_yield_in_main() {
        Coroutine::sched();
    }

    #[bench]
    fn bench_coroutine_spawning_with_recycle(b: &mut Bencher) {
        b.iter(|| {
            let _ = Coroutine::spawn(move|| {}).resume();
        });
    }

    #[bench]
    fn bench_normal_counting(b: &mut Bencher) {
        b.iter(|| {
            const MAX_NUMBER: usize = 100;

            let (tx, rx) = channel();

            let mut result = 0;
            for _ in 0..MAX_NUMBER {
                tx.send(1).unwrap();
                result += rx.recv().unwrap();
            }
            assert_eq!(result, MAX_NUMBER);
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

            let _ = coro.resume();

            let result = rx.iter().fold(0, |a, b| {
                let _ = Coroutine::resume(&coro);
                a + b
            });
            assert_eq!(result, MAX_NUMBER);
        });
    }
}
