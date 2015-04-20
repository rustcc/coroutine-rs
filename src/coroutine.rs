// The MIT License (MIT)

// Copyright (c) 2015 Rustcc developers

// Permission is hereby granted, free of charge, to any person obtaining a copy of
// this software and associated documentation files (the "Software"), to deal in
// the Software without restriction, including without limitation the rights to
// use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of
// the Software, and to permit persons to whom the Software is furnished to do so,
// subject to the following conditions:

// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.

// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS
// FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR
// COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER
// IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
// CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

//! Basic single threaded Coroutine
//!
//! ```rust
//! use coroutine::{spawn, sched};
//!
//! let coro = spawn(|| {
//!     println!("Before yield");
//!
//!     // Yield back to its parent who resume this coroutine
//!     sched();
//!
//!     println!("I am back!");
//! });
//!
//! // Starts the Coroutine
//! coro.resume().ok().expect("Failed to resume");
//!
//! println!("Back to main");
//!
//! // Resume it
//! coro.resume().ok().expect("Failed to resume");
//!
//! println!("Coroutine finished");
//! ```

use std::default::Default;
use std::rt::util::min_stack;
use thunk::Thunk;
use std::mem::transmute;
use std::rt::unwind::try;
use std::any::Any;
use std::cell::UnsafeCell;
use std::ops::{Deref, DerefMut};
use std::ptr;
use std::sync::Arc;
use std::cell::RefCell;

use context::Context;
use stack::{StackPool, Stack};

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

/// Return type of resuming.
///
/// See `Coroutine::resume` for more detail
pub type ResumeResult<T> = Result<T, Box<Any + Send>>;

/// Coroutine spawn options
#[derive(Debug)]
pub struct Options {
    /// The size of the stack
    pub stack_size: usize,

    /// The name of the Coroutine
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
#[derive(Debug, Clone)]
pub struct Handle(Arc<RefCell<Box<Coroutine>>>);

unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}

impl Handle {
    fn new(c: Box<Coroutine>) -> Handle {
        Handle(Arc::new(RefCell::new(c)))
    }

    /// Resume the Coroutine
    pub fn resume(&self) -> ResumeResult<()> {
        match self.state() {
            State::Finished | State::Running => return Ok(()),
            State::Panicked => panic!("Trying to resume a panicked coroutine"),
            _ => {}
        }

        COROUTINE_ENVIRONMENT.with(|env| {
            let env: &mut Environment = unsafe { transmute(env.get()) };

            let from_coro_hdl = Coroutine::current();
            let from_coro: &mut Coroutine = unsafe {
                let c: &mut Box<Coroutine> = transmute(from_coro_hdl.as_unsafe_cell().get());
                c.deref_mut()
            };

            let to_coro: &mut Box<Coroutine> = unsafe {
                transmute(self.as_unsafe_cell().get())
            };

            // Save state
            to_coro.set_state(State::Running);
            to_coro.parent = from_coro;
            from_coro.set_state(State::Normal);

            env.current_running = self.clone();
            Context::swap(&mut from_coro.saved_context, &to_coro.saved_context);
            env.current_running = from_coro_hdl;

            match env.running_state.take() {
                Some(err) => Err(err),
                None => Ok(()),
            }
        })
    }

    /// Join this Coroutine.
    ///
    /// If the Coroutine panicked, this method will return an `Err` with panic message.
    ///
    /// ```ignore
    /// // Wait until the Coroutine exits
    /// spawn(|| {
    ///     println!("Before yield");
    ///     sched();
    ///     println!("Exiting");
    /// }).join().unwrap();
    /// ```
    #[inline]
    pub fn join(&self) -> ResumeResult<()> {
        loop {
            match self.state() {
                State::Suspended => try!(self.resume()),
                _ => break,
            }
        }
        Ok(())
    }

    /// Get the state of the Coroutine
    #[inline]
    pub fn state(&self) -> State {
        unsafe {
            let c: &mut Box<Coroutine> = transmute(self.as_unsafe_cell().get());
            c.state()
        }
    }

    /// Set the state of the Coroutine
    #[inline]
    pub fn set_state(&self, state: State) {
        unsafe {
            let c: &mut Box<Coroutine> = transmute(self.as_unsafe_cell().get());
            c.set_state(state)
        }
    }

}

impl Deref for Handle {
    type Target = <Arc<RefCell<Box<Coroutine>>> as Deref>::Target;

    #[inline]
    fn deref(&self) -> &<Arc<RefCell<Box<Coroutine>>> as Deref>::Target {
        self.0.deref()
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

    let state = COROUTINE_ENVIRONMENT.with(move|env| {
        let env: &mut Environment = unsafe { transmute(env.get()) };

        let cur: &mut Box<Coroutine> = unsafe {
            transmute(env.current_running.as_unsafe_cell().get())
        };

        match ret {
            Ok(..) => {
                env.running_state = None;

                State::Finished
            }
            Err(err) => {
                {
                    use std::io::stderr;
                    use std::io::Write;
                    let msg = match err.downcast_ref::<&'static str>() {
                        Some(s) => *s,
                        None => match err.downcast_ref::<String>() {
                            Some(s) => &s[..],
                            None => "Box<Any>",
                        }
                    };

                    let name = cur.name().unwrap_or("<unnamed>");

                    let _ = writeln!(&mut stderr(), "Coroutine '{}' panicked at '{}'", name, msg);
                }

                env.running_state = Some(err);

                State::Panicked
            }
        }
    });

    loop {
        Coroutine::yield_now(state);
    }
}

impl Coroutine {
    unsafe fn empty(name: Option<String>, state: State) -> Handle {
        Handle::new(box Coroutine {
            current_stack_segment: None,
            saved_context: Context::empty(),
            parent: ptr::null_mut(),
            state: state,
            name: name,
        })
    }

    fn new(name: Option<String>, stack: Stack, ctx: Context, state: State) -> Handle {
        Handle::new(box Coroutine {
            current_stack_segment: Some(stack),
            saved_context: ctx,
            parent: ptr::null_mut(),
            state: state,
            name: name,
        })
    }

    /// Spawn a Coroutine with options
    pub fn spawn_opts<F>(f: F, opts: Options) -> Handle
            where F: FnOnce() + Send + 'static {

        COROUTINE_ENVIRONMENT.with(move|env| {
            unsafe {
                let env: &mut Environment = transmute(env.get());

                let mut stack = env.stack_pool.take_stack(opts.stack_size);

                let ctx = Context::new(coroutine_initialize,
                                   0,
                                   f,
                                   &mut stack);

                Coroutine::new(opts.name, stack, ctx, State::Suspended)
            }
        })
    }

    /// Spawn a Coroutine with default options
    pub fn spawn<F>(f: F) -> Handle
            where F: FnOnce() + Send + 'static {
        Coroutine::spawn_opts(f, Default::default())
    }

    /// Yield the current running Coroutine
    #[inline]
    pub fn yield_now(state: State) {
        // Cannot yield with Running state
        assert!(state != State::Running);

        COROUTINE_ENVIRONMENT.with(|env| unsafe {
            let env: &mut Environment = transmute(env.get());

            let from_coro: &mut Box<Coroutine> = {
                transmute(env.current_running.as_unsafe_cell().get())
            };

            from_coro.set_state(state);

            let to_coro: &mut Coroutine = transmute(from_coro.parent);
            Context::swap(&mut from_coro.saved_context, &to_coro.saved_context);
        })
    }

    /// Yield the current running Coroutine with `Suspended` state
    #[inline]
    pub fn sched() {
        Coroutine::yield_now(State::Suspended)
    }

    /// Yield the current running Coroutine with `Blocked` state
    #[inline]
    pub fn block() {
        Coroutine::yield_now(State::Blocked)
    }

    /// Get a Handle to the current running Coroutine.
    ///
    /// It is unsafe because it is an undefined behavior if you resume a Coroutine
    /// in more than one native thread.
    #[inline]
    pub fn current() -> Handle {
        COROUTINE_ENVIRONMENT.with(move|env| unsafe {
            let env: &mut Environment = transmute(env.get());
            env.current_running.clone()
        })
    }

    #[inline(always)]
    fn state(&self) -> State {
        self.state
    }

    #[inline(always)]
    fn set_state(&mut self, state: State) {
        self.state = state
    }

    /// Get the name of the Coroutine
    #[inline(always)]
    pub fn name(&self) -> Option<&str> {
        self.name.as_ref().map(|s| &**s)
    }
}

thread_local!(static COROUTINE_ENVIRONMENT: UnsafeCell<Environment> = UnsafeCell::new(Environment::new()));

/// Coroutine managing environment
#[allow(raw_pointer_derive)]
struct Environment {
    stack_pool: StackPool,

    current_running: Handle,
    __main_coroutine: Handle,

    running_state: Option<Box<Any + Send>>,
}

impl Environment {
    /// Initialize a new environment
    fn new() -> Environment {
        let coro = unsafe {
            let coro = Coroutine::empty(Some("<Environment Root Coroutine>".to_string()), State::Running);
            coro.borrow_mut().parent = {
                let itself: &mut Box<Coroutine> = transmute(coro.as_unsafe_cell().get());
                itself.deref_mut()
            }; // Point to itself
            coro
        };

        Environment {
            stack_pool: StackPool::new(),
            current_running: coro.clone(),
            __main_coroutine: coro,

            running_state: None,
        }
    }
}

/// Coroutine configuration. Provides detailed control over the properties and behavior of new Coroutines.
///
/// ```ignore
/// let coro = Builder::new().name(format!("Coroutine #{}", 1))
///                          .stack_size(4096)
///                          .spawn(|| println!("Hello world!!"));
///
/// coro.resume().unwrap();
/// ```
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
///
/// Equavalent to `Coroutine::spawn`.
pub fn spawn<F>(f: F) -> Handle
        where F: FnOnce() + Send + 'static {
    Builder::new().spawn(f)
}

/// Get the current Coroutine
///
/// Equavalent to `Coroutine::current`.
pub fn current() -> Handle {
    Coroutine::current()
}

/// Yield the current Coroutine
///
/// Equavalent to `Coroutine::sched`.
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
        }).resume().ok().expect("Failed to resume");

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
        coro.resume().ok().expect("Failed to resume");
        assert_eq!(rx.recv().unwrap(), 1);
        assert!(rx.try_recv().is_err());

        coro.resume().ok().expect("Failed to resume");

        assert_eq!(rx.recv().unwrap(), 2);
    }

    #[test]
    fn test_coroutine_spawn_inside() {
        let (tx, rx) = channel();
        Coroutine::spawn(move|| {
            tx.send(1).unwrap();

            Coroutine::spawn(move|| {
                tx.send(2).unwrap();
            }).join().ok().expect("Failed to join");

        }).join().ok().expect("Failed to join");

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
        }).join().ok().expect("Failed to join");
    }

    #[test]
    fn test_coroutine_resume_after_finished() {
        let coro = Coroutine::spawn(move|| {});

        // It is already finished, but we try to resume it
        // Idealy it would come back immediately
        assert!(coro.resume().is_ok());

        // Again?
        assert!(coro.resume().is_ok());
    }

    #[test]
    fn test_coroutine_resume_itself() {
        let coro = Coroutine::spawn(move|| {
            // Resume itself
            Coroutine::current().resume().ok().expect("Failed to resume");
        });

        assert!(coro.resume().is_ok());
    }

    #[test]
    fn test_coroutine_yield_in_main() {
        Coroutine::sched();
    }

    #[test]
    fn test_builder_basic() {
        let (tx, rx) = channel();
        Builder::new().name("Test builder".to_string()).spawn(move|| {
            tx.send(1).unwrap();
        }).join().ok().expect("Failed to join");
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

            let coro = Coroutine::spawn(move|| {
                for _ in 0..MAX_NUMBER {
                    tx.send(1).unwrap();
                    Coroutine::sched();
                }
            });
            coro.resume().ok().expect("Failed to resume");

            let mut result = 0;
            for n in rx.iter() {
                coro.resume().ok().expect("Failed to resume");
                result += n;
            }
            assert_eq!(result, MAX_NUMBER);
        });
    }
}
