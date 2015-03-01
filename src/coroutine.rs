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
use std::mem::{transmute, transmute_copy};
use std::rt::unwind::try;
use std::any::Any;
use std::cell::UnsafeCell;
use std::ops::DerefMut;
use std::ptr;

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
    pub stack_size: usize,
    pub name: Option<String>,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            stack_size: min_stack(),
            name: None,
        }
    }
}

/// Handle of a Coroutine
#[derive(Debug)]
pub struct Handle(Box<Coroutine>);

impl Handle {

    #[inline]
    pub fn state(&self) -> State {
        self.0.state()
    }

    pub fn resume(mut self) -> ResumeResult<Handle> {
        match self.state() {
            State::Finished | State::Running => return Ok(self),
            State::Panicked => panic!("Tried to resume a panicked coroutine"),
            _ => {}
        }

        COROUTINE_ENVIRONMENT.with(|env| {
            let env: &mut Environment = unsafe { transmute(env.get()) };

            // Move out, keep this reference
            // It may not be the only reference to the Coroutine
            //
            let mut from_coro = env.current_running.take().unwrap();

            // Save state
            self.0.state = State::Running;
            self.0.parent = from_coro.0.deref_mut();
            from_coro.0.state = State::Suspended;

            // Move inside the Environment
            env.current_running = Some(self);

            Context::swap(&mut from_coro.0.saved_context, &env.current_running.as_ref().unwrap().0.saved_context);

            // Move out here
            self = env.current_running.take().unwrap();
            env.current_running = Some(from_coro);
            
            match env.running_state.take() {
                Some(err) => {
                    Err(err)
                },
                None => {
                    Ok(self)
                }
            }
        })
    }

    #[inline]
    pub fn join(self) -> ResumeResult<Handle> {
        let mut coro = self;
        loop {
            match coro.state() {
                State::Suspended => coro = try!(coro.resume()),
                _ => break,
            }
        }
        Ok(coro)
    }
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

    /// Parent coroutine, will always be valid
    parent: *mut Coroutine,

    /// State
    state: State,

    /// Name
    name: Option<String>,
}

unsafe impl Send for Coroutine {}
unsafe impl Sync for Coroutine {}

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

        match ret {
            Ok(..) => {
                env.running_state = None;
                env.current_running.as_mut().unwrap().0.state = State::Finished;
            }
            Err(err) => {
                env.running_state = Some(err);
                env.current_running.as_mut().unwrap().0.state = State::Panicked;
            }
        }
    });

    loop {
        Coroutine::sched();
    }
}

impl Coroutine {
    pub fn empty() -> Handle {
        Handle(box Coroutine {
            current_stack_segment: None,
            saved_context: Context::empty(),
            parent: ptr::null_mut(),
            state: State::Running,
            name: None,
        })
    }

    pub fn with_name(name: &str) -> Handle {
        Handle(box Coroutine {
            current_stack_segment: None,
            saved_context: Context::empty(),
            parent: ptr::null_mut(),
            state: State::Running,
            name: Some(name.to_string()),
        })
    }

    pub fn spawn_opts<F>(f: F, opts: Options) -> Handle
            where F: FnOnce() + Send + 'static {

        let coro = UnsafeCell::new(Coroutine::empty());
        COROUTINE_ENVIRONMENT.with(|env| {
            unsafe {
                let env: &mut Environment = transmute(env.get());

                let mut stack = env.stack_pool.take_stack(opts.stack_size);

                let ctx = Context::new(coroutine_initialize,
                                   0,
                                   f,
                                   &mut stack);

                let coro: &mut Handle = transmute(coro.get());
                coro.0.saved_context = ctx;
                coro.0.current_stack_segment = Some(stack);
                coro.0.state = State::Suspended;
            }
        });

        let mut coro = unsafe { coro.into_inner() };
        coro.0.name = opts.name;
        coro
    }

    /// Spawn a coroutine with default options
    pub fn spawn<F>(f: F) -> Handle
            where F: FnOnce() + Send + 'static {
        Coroutine::spawn_opts(f, Default::default())
    }

    pub fn sched() {
        COROUTINE_ENVIRONMENT.with(|env| {
            let env: &mut Environment = unsafe { transmute(env.get()) };

            // Move out
            let mut from_coro = env.current_running.as_mut().unwrap();

            match from_coro.state() {
                State::Finished | State::Panicked => {},
                _ => from_coro.0.state = State::Suspended,
            }

            let to_coro: &mut Coroutine = unsafe { transmute_copy(&from_coro.0.parent) };

            Context::swap(&mut from_coro.0.saved_context, &to_coro.saved_context);
        });
    }

    pub fn current() -> HandleGuard {
        HandleGuard
    }

    pub fn state(&self) -> State {
        self.state
    }
}

pub struct HandleGuard;

impl HandleGuard {
    pub fn with<F>(&self, f: F)
            where F: FnOnce(Handle) -> Handle + Send + 'static {
        COROUTINE_ENVIRONMENT.with(|env| {
            let env: &mut Environment = unsafe { transmute(env.get()) };
            env.current_running = Some(f(env.current_running.take().unwrap()));
        });
    }
}

thread_local!(static COROUTINE_ENVIRONMENT: UnsafeCell<Environment> = UnsafeCell::new(Environment::new()));

/// Coroutine managing environment
#[allow(raw_pointer_derive)]
#[derive(Debug)]
struct Environment {
    stack_pool: StackPool,
    current_running: Option<Handle>,

    running_state: Option<Box<Any + Send>>,
}

impl Environment {
    /// Initialize a new environment
    fn new() -> Environment {
        let mut coro = Coroutine::with_name("<Environment Root Coroutine>");
        coro.0.parent = coro.0.deref_mut(); // Pointing to itself

        Environment {
            stack_pool: StackPool::new(),
            current_running: Some(coro),

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
        Coroutine::spawn(move|| {
            tx.send(1).unwrap();
        }).resume().unwrap();

        assert_eq!(rx.recv().unwrap(), 1);
    }

    #[test]
    fn test_coroutine_yield() {
        let (tx, rx) = channel();
        let coro = Coroutine::spawn(move|| {
            tx.send(1).unwrap();

            Coroutine::sched();

            tx.send(2).unwrap();
        }).resume().unwrap();
        assert_eq!(rx.recv().unwrap(), 1);
        assert!(rx.try_recv().is_err());

        coro.resume().unwrap();

        assert_eq!(rx.recv().unwrap(), 2);
    }

    #[test]
    fn test_coroutine_spawn_inside() {
        let (tx, rx) = channel();
        Coroutine::spawn(move|| {
            tx.send(1).unwrap();

            Coroutine::spawn(move|| {
                tx.send(2).unwrap();
            }).join().unwrap();

        }).join().unwrap();;

        assert_eq!(rx.recv().unwrap(), 1);
        assert_eq!(rx.recv().unwrap(), 2);
    }

    #[test]
    fn test_coroutine_panic() {
        let coro = Coroutine::spawn(move|| {
            panic!("Panic inside a coroutine!!");
        });
        assert!(coro.join().is_err());
    }

    #[test]
    fn test_coroutine_child_panic() {
        Coroutine::spawn(move|| {
            let _ = Coroutine::spawn(move|| {
                panic!("Panic inside a coroutine's child!!");
            }).join();
        }).join().unwrap();
    }

    #[test]
    fn test_coroutine_resume_after_finished() {
        let mut coro = Coroutine::spawn(move|| {});

        // It is already finished, but we try to resume it
        // Idealy it would come back immediately
        coro = coro.resume().unwrap();

        // Again?
        assert!(coro.resume().is_ok());
    }

    #[test]
    fn test_coroutine_resume_itself() {
        let coro = Coroutine::spawn(move|| {
            // Resume itself
            Coroutine::current().with(|me| {
                me.resume().unwrap()
            });
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

            let mut coro = Coroutine::spawn(move|| {
                for _ in 0..MAX_NUMBER {
                    tx.send(1).unwrap();
                    Coroutine::sched();
                }
            }).resume().unwrap();;

            let mut result = 0;
            for n in rx.iter() {
                coro = coro.resume().unwrap();
                result += n;
            }
            assert_eq!(result, MAX_NUMBER);
        });
    }
}
