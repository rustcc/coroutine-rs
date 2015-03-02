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
pub type Handle = Box<Coroutine>;

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
        let cur: &mut Coroutine = unsafe { transmute(env.current_running) };

        match ret {
            Ok(..) => {
                env.running_state = None;
                cur.state = State::Finished;
            }
            Err(err) => {
                cur.state = State::Panicked;

                {
                    use std::rt::util::Stderr;
                    let msg = match err.downcast_ref::<&'static str>() {
                        Some(s) => *s,
                        None => match err.downcast_ref::<String>() {
                            Some(s) => &s[..],
                            None => "Box<Any>",
                        }
                    };

                    let name = cur.name().unwrap_or("<unnamed>");

                    let mut stderr = Stderr;
                    let _ = writeln!(&mut stderr, "Coroutine '{}' panicked at '{}'", name, msg);
                }

                env.running_state = Some(err);
            }
        }
    });

    loop {
        Coroutine::sched();
    }
}

impl Coroutine {
    fn empty() -> Handle {
        box Coroutine {
            current_stack_segment: None,
            saved_context: Context::empty(),
            parent: ptr::null_mut(),
            state: State::Running,
            name: None,
        }
    }

    fn with_name(name: &str) -> Handle {
        box Coroutine {
            current_stack_segment: None,
            saved_context: Context::empty(),
            parent: ptr::null_mut(),
            state: State::Running,
            name: Some(name.to_string()),
        }
    }

    pub fn spawn_opts<F>(f: F, opts: Options) -> Handle
            where F: FnOnce() + Send + 'static {

        let mut coro = COROUTINE_ENVIRONMENT.with(|env| {
            unsafe {
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
            }
        });

        coro.name = opts.name;
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

            let from_coro: &mut Coroutine = unsafe { transmute(env.current_running) };

            match from_coro.state() {
                State::Finished | State::Panicked => {},
                _ => from_coro.state = State::Suspended,
            }

            let to_coro: &mut Coroutine = unsafe { transmute(from_coro.parent) };
            to_coro.state = State::Running;
            env.current_running = from_coro.parent;

            Context::swap(&mut from_coro.saved_context, &to_coro.saved_context);
        });
    }

    pub fn resume(&mut self) -> ResumeResult<()> {
        match self.state() {
            State::Finished | State::Running => return Ok(()),
            State::Panicked => panic!("Tried to resume a panicked coroutine"),
            _ => {}
        }

        let mut from_coro: &mut Coroutine = COROUTINE_ENVIRONMENT.with(|env| {
            unsafe {
                let env: &mut Environment = transmute(env.get());
                let cur = env.current_running;

                // Move inside the Environment
                env.current_running = transmute_copy(&self);

                transmute(cur)
            }
        });

        // Save state
        self.state = State::Running;
        self.parent = from_coro;
        from_coro.state = State::Suspended;

        Context::swap(&mut from_coro.saved_context, &self.saved_context);

        COROUTINE_ENVIRONMENT.with(|env| {
            let env: &mut Environment = unsafe { transmute(env.get()) };
            match env.running_state.take() {
                Some(err) => Err(err),
                None => Ok(()),
            }
        })
    }

    pub fn current() -> &'static mut Coroutine {
        COROUTINE_ENVIRONMENT.with(|env| {
            unsafe {
                let env: &mut Environment = transmute(env.get());
                transmute(env.current_running)
            }
        })
    }

    pub fn join(&mut self) -> ResumeResult<()> {
        loop {
            match self.state() {
                State::Suspended => try!(self.resume()),
                _ => break,
            }
        }
        Ok(())
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn name(&self) -> Option<&str> {
        self.name.as_ref().map(|s| &**s)
    }
}

thread_local!(static COROUTINE_ENVIRONMENT: UnsafeCell<Environment> = UnsafeCell::new(Environment::new()));

/// Coroutine managing environment
#[allow(raw_pointer_derive)]
#[derive(Debug)]
struct Environment {
    stack_pool: StackPool,
    current_running: *mut Coroutine,
    __main_coroutine: Handle,

    running_state: Option<Box<Any + Send>>,
}

impl Environment {
    /// Initialize a new environment
    fn new() -> Environment {
        let mut coro = Coroutine::with_name("<Environment Root Coroutine>");
        coro.parent = coro.deref_mut(); // Pointing to itself

        Environment {
            stack_pool: StackPool::new(),
            current_running: coro.deref_mut(),
            __main_coroutine: coro,

            running_state: None,
        }
    }
}

/// Coroutine configuration. Provides detailed control over the properties and behavior of new Coroutines.
pub struct Builder {
    opts: Options,
}

impl Builder {
    /// Generate the base configuration for spawning a Coroutine, from which configuration methods can be chained.
    pub fn new() -> Builder {
        Builder {
            opts: Default::default(),
        }
    }

    /// Name the Coroutine-to-be. Currently the name is used for identification only in panic messages.
    pub fn name(mut self, name: String) -> Builder {
        self.opts.name = Some(name);
        self
    }

    /// Set the size of the stack for the new Coroutine.
    pub fn stack_size(mut self, size: usize) -> Builder {
        self.opts.stack_size = size;
        self
    }

    /// Spawn a new Coroutine, and return a handle for it.
    pub fn spawn<F>(self, f: F) -> Handle
            where F: FnOnce() + Send + 'static {
        Coroutine::spawn_opts(f, self.opts)
    }
}

/// Spawn a new Coroutine
pub fn spawn<F>(f: F) -> Handle
        where F: FnOnce() + Send + 'static {
    Builder::new().spawn(f)
}

// /// Get the current Coroutine
pub fn current() -> &'static mut Coroutine {
    Coroutine::current()
}

/// Yield the current Coroutine
pub fn sched() {
    Coroutine::sched()
}

#[cfg(test)]
mod test {
    use std::sync::mpsc::channel;

    use super::Coroutine;
    use super::Builder;

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
        let mut coro = Coroutine::spawn(move|| {
            tx.send(1).unwrap();

            Coroutine::sched();

            tx.send(2).unwrap();
        });
        coro.resume().unwrap();
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
        let mut coro = Coroutine::spawn(move|| {
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
        assert!(coro.resume().is_ok());

        // Again?
        assert!(coro.resume().is_ok());
    }

    // #[test]
    // fn test_coroutine_resume_itself() {
    //     let coro = Coroutine::spawn(move|| {
    //         // Resume itself
    //         Coroutine::current().with(|me| {
    //             me.resume().unwrap()
    //         });
    //     });

    //     assert!(coro.resume().is_ok());
    // }

    #[test]
    fn test_coroutine_yield_in_main() {
        Coroutine::sched();
    }

    #[test]
    fn test_builder_basic() {
        let (tx, rx) = channel();
        Builder::new().name("Test builder".to_string()).spawn(move|| {
            tx.send(1).unwrap();
        }).join().unwrap();
        assert_eq!(rx.recv().unwrap(), 1);
    }
}

#[cfg(test)]
mod bench {
    use std::sync::mpsc::channel;

    use test::Bencher;

    use super::Coroutine;

    #[bench]
    fn bench_coroutine_spawning_with_recycle(b: &mut Bencher) {
        b.iter(|| {
            let _ = Coroutine::spawn(move|| {}).join();
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
            });
            coro.resume().unwrap();;

            let mut result = 0;
            for n in rx.iter() {
                coro.resume().unwrap();
                result += n;
            }
            assert_eq!(result, MAX_NUMBER);
        });
    }
}
