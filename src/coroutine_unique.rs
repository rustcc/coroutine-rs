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
//!

/* Here is the coroutine(with scheduler) workflow:
 *
 *                               --------------------------------
 * --------------------------    |                              |
 * |                        |    v                              |
 * |                  ----------------                          |  III.Coroutine::yield_now()
 * |             ---> |   Scheduler  |  <-----                  |
 * |    parent   |    ----------------       |   parent         |
 * |             |           ^ parent        |                  |
 * |   --------------  --------------  --------------           |
 * |   |Coroutine(1)|  |Coroutine(2)|  |Coroutine(3)|  ----------
 * |   --------------  --------------  --------------
 * |         ^            |     ^
 * |         |            |     |  II.do_some_works
 * -----------            -------
 *   I.Handle.resume()
 *
 *
 *  First, all coroutines have a link to a parent coroutine, which was set when the coroutine resumed.
 *  In the scheduler/coroutine model, every worker coroutine has a parent pointer pointing to
 *  the scheduler coroutine(which is a raw thread).
 *  Scheduler resumes a proper coroutine and set the parent pointer, like procedure I does.
 *  When a coroutine is awaken, it does some work like procedure II does.
 *  When a coroutine yield(io, finished, paniced or sched), it resumes its parent's context,
 *  like procedure III does.
 *  Now the scheduler is awake again and it simply decides whether to put the coroutine to queue again or not,
 *  according to the coroutine's return status.
 *  And last, the scheduler continues the scheduling loop and selects a proper coroutine to wake up.
 */

use std::default::Default;
use std::rt::util::min_stack;
use thunk::Thunk;
use std::mem::transmute;
use std::rt::unwind::try;
use std::any::Any;
use std::cell::UnsafeCell;
use std::ops::Deref;
use std::ptr::Unique;

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
pub struct Handle(Unique<Coroutine>);

unsafe impl Send for Handle {}

impl Handle {
    fn new(c: Coroutine) -> Handle {
        unsafe {
            Handle(Unique::new(transmute(Box::new(c))))
        }
    }

    unsafe fn get_inner_mut(&self) -> &mut Coroutine {
        &mut **self.0
    }

    unsafe fn get_inner(&self) -> &Coroutine {
        self.0.get()
    }

    /// Resume the Coroutine
    pub fn resume(&self) -> ResumeResult<()> {
        match self.state() {
            State::Finished | State::Running => return Ok(()),
            State::Panicked => panic!("Trying to resume a panicked coroutine"),
            State::Normal => panic!("Coroutine {:?} is waiting for its child to return, cannot resume!",
                                    self.name().unwrap_or("<unnamed>")),
            _ => {}
        }

        let env = Environment::current();

        let from_coro_hdl = Coroutine::current();
        {
            let (from_coro, to_coro) = unsafe {
                (from_coro_hdl.get_inner_mut(), self.get_inner_mut())
            };

            // Save state
            to_coro.set_state(State::Running);
            from_coro.set_state(State::Normal);

            env.coroutine_stack.push(unsafe { transmute(self) });
            Context::swap(&mut from_coro.saved_context, &to_coro.saved_context);
        }

        match env.running_state.take() {
            Some(err) => Err(err),
            None => Ok(()),
        }
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
                State::Suspended | State::Blocked => try!(self.resume()),
                _ => break,
            }
        }
        Ok(())
    }

    /// Get the state of the Coroutine
    #[inline]
    pub fn state(&self) -> State {
        unsafe { self.get_inner().state() }
    }

    /// Set the state of the Coroutine
    #[inline]
    pub fn set_state(&self, state: State) {
        unsafe { self.get_inner_mut().set_state(state) }
    }
}

impl Deref for Handle {
    type Target = Coroutine;

    #[inline]
    fn deref(&self) -> &Coroutine {
        unsafe { self.get_inner() }
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

    /// State
    state: State,

    /// Name
    name: Option<String>,
}

unsafe impl Send for Coroutine {}

/// Destroy coroutine and try to reuse std::stack segment.
impl Drop for Coroutine {
    fn drop(&mut self) {
        match self.current_stack_segment.take() {
            Some(stack) => {
                let env = Environment::current();
                env.stack_pool.give_stack(stack);
            },
            None => {}
        }
    }
}

/// Initialization function for make context
extern "C" fn coroutine_initialize(_: usize, f: *mut ()) -> ! {
    let func: Box<Thunk> = unsafe { transmute(f) };

    let ret = unsafe { try(move|| func.invoke(())) };

    let env = Environment::current();

    let cur: &mut Coroutine = unsafe {
        let last = & **env.coroutine_stack.last().expect("Impossible happened! No current coroutine!");
        last.get_inner_mut()
    };

    let state = match ret {
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
    };

    loop {
        Coroutine::yield_now(state);
    }
}

impl Coroutine {
    unsafe fn empty(name: Option<String>, state: State) -> Handle {
        Handle::new(Coroutine {
            current_stack_segment: None,
            saved_context: Context::empty(),
            state: state,
            name: name,
        })
    }

    fn new(name: Option<String>, stack: Stack, ctx: Context, state: State) -> Handle {
        Handle::new(Coroutine {
            current_stack_segment: Some(stack),
            saved_context: ctx,
            state: state,
            name: name,
        })
    }

    /// Spawn a Coroutine with options
    pub fn spawn_opts<F>(f: F, opts: Options) -> Handle
        where F: FnOnce() + Send + 'static
    {

        let env = Environment::current();
        let mut stack = env.stack_pool.take_stack(opts.stack_size);

        let ctx = Context::new(coroutine_initialize, 0, f, &mut stack);

        Coroutine::new(opts.name, stack, ctx, State::Suspended)
    }

    /// Spawn a Coroutine with default options
    pub fn spawn<F>(f: F) -> Handle
        where F: FnOnce() + Send + 'static
    {
        Coroutine::spawn_opts(f, Default::default())
    }

    /// Yield the current running Coroutine to its parent
    #[inline]
    pub fn yield_now(state: State) {
        // Cannot yield with Running state
        assert!(state != State::Running);

        let env = Environment::current();
        if env.coroutine_stack.len() == 1 {
            // Environment root
            return;
        }

        unsafe {
            match (env.coroutine_stack.pop(), env.coroutine_stack.last()) {
                (Some(from_coro), Some(to_coro)) => {
                    (&mut *from_coro).set_state(state);
                    Context::swap(&mut (& *from_coro).get_inner_mut().saved_context, &(& **to_coro).saved_context);
                },
                _ => unreachable!()
            }
        }
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
    pub fn current() -> &'static Handle {
        Environment::current().coroutine_stack.last().map(|hdl| unsafe { (& **hdl) })
            .expect("Impossible happened! No current coroutine!")
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

thread_local!(static COROUTINE_ENVIRONMENT: UnsafeCell<Box<Environment>> = UnsafeCell::new(Environment::new()));

/// Coroutine managing environment
#[allow(raw_pointer_derive)]
struct Environment {
    stack_pool: StackPool,

    coroutine_stack: Vec<*mut Handle>,
    _main_coroutine: Handle,

    running_state: Option<Box<Any + Send>>,
}

impl Environment {
    /// Initialize a new environment
    fn new() -> Box<Environment> {
        let coro = unsafe {
            let coro = Coroutine::empty(Some("<Environment Root Coroutine>".to_string()), State::Running);
            coro
        };

        let mut env = Box::new(Environment {
            stack_pool: StackPool::new(),

            coroutine_stack: Vec::new(),
            _main_coroutine: coro,

            running_state: None,
        });

        let coro: *mut Handle = &mut env._main_coroutine;
        env.coroutine_stack.push(coro);

        env
    }

    fn current() -> &'static mut Environment {
        COROUTINE_ENVIRONMENT.with(|env| unsafe {
            &mut *env.get()
        })
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
        where F: FnOnce() + Send + 'static
    {
        Coroutine::spawn_opts(f, self.opts)
    }
}

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
pub fn resume(coro: &Handle) -> ResumeResult<()> {
    coro.resume()
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
    use test::Bencher;

    use super::Coroutine;

    #[bench]
    fn bench_coroutine_spawning(b: &mut Bencher) {
        b.iter(|| {
            let _ = Coroutine::spawn(move|| {});
        });
    }

    #[bench]
    fn bench_context_switch(b: &mut Bencher) {
        let coro = Coroutine::spawn(|| {
            loop {
                // 3. Save current context
                // 4. Switch
                Coroutine::sched();
            }
        });

        b.iter(|| {
            // 1. Save current context
            // 2. Switch
            let _ = coro.resume();
        });
    }
}
