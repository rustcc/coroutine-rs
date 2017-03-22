// The MIT License (MIT)

// Copyright (c) 2015 Y. T. Chung <zonyitoo@gmail.com>

//  Permission is hereby granted, free of charge, to any person obtaining a
//  copy of this software and associated documentation files (the "Software"),
//  to deal in the Software without restriction, including without limitation
//  the rights to use, copy, modify, merge, publish, distribute, sublicense,
//  and/or sell copies of the Software, and to permit persons to whom the
//  Software is furnished to do so, subject to the following conditions:
//
//  The above copyright notice and this permission notice shall be included in
//  all copies or substantial portions of the Software.
//
//  THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
//  OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
//  FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
//  AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
//  LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
//  FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
//  DEALINGS IN THE SOFTWARE.

//! Asymmetric coroutines

use std::boxed::FnBox;
use std::fmt;
use std::usize;
use std::panic;
use std::mem;
use std::iter::Iterator;
use std::any::Any;

use context::{Context, Transfer};
use context::stack::ProtectedFixedSizeStack;

use options::Options;

#[derive(Debug)]
struct ForceUnwind;

type Thunk<'a> = Box<FnBox(&mut Coroutine, usize) -> usize + 'a>;

struct InitData {
    stack: ProtectedFixedSizeStack,
    callback: Thunk<'static>,
}

extern "C" fn coroutine_entry(t: Transfer) -> ! {
    // Take over the data from Coroutine::spawn_opts
    let InitData { stack, callback } = unsafe {
        let data_opt_ref = &mut *(t.data as *mut Option<InitData>);
        data_opt_ref.take().expect("failed to acquire InitData")
    };

    // This block will ensure the `meta` will be destroied before dropping the stack
    let (ctx, result) = {
        let mut meta = Coroutine {
            context: None,
            name: None,
            state: State::Suspended,
            panicked_error: None,
        };

        // Yield back after take out the callback function
        // Now the Coroutine is initialized
        let meta_ptr = &mut meta as *mut _ as usize;
        let result = unsafe {
            ::try(move || {
                let Transfer { context, data } = t.context.resume(meta_ptr);
                let meta_ref = &mut *(meta_ptr as *mut Coroutine);
                meta_ref.context = Some(context);

                // Take out the callback and run it
                // let result = callback.call_box((meta_ref, data));
                let result = callback.call_box((meta_ref, data));

                trace!("Coroutine `{}`: returned from callback with result {}",
                       meta_ref.debug_name(),
                       result);
                result
            })
        };

        let mut loc_data = match result {
            Ok(d) => {
                meta.state = State::Finished;
                d
            }
            Err(err) => {
                if err.is::<ForceUnwind>() {
                    meta.state = State::Finished
                } else {
                    meta.state = State::Panicked;
                    meta.panicked_error = Some(err);
                }
                usize::MAX
            }
        };

        trace!("Coroutine `{}`: exited with {:?}",
               meta.debug_name(),
               meta.state);

        loop {
            let Transfer { context, data } = meta.context.take().unwrap().resume(loc_data);
            meta.context = Some(context);
            loc_data = data;

            if meta.state == State::Finished {
                break;
            }
        }

        trace!("Coroutine `{}`: finished => dropping stack",
               meta.debug_name());

        // If panicked inside, the meta.context stores the actual return Context
        (meta.take_context(), loc_data)
    };

    // Drop the stack after it is finished
    let mut stack_opt = Some((stack, result));
    ctx.resume_ontop(&mut stack_opt as *mut _ as usize, coroutine_exit);

    unreachable!();
}

extern "C" fn coroutine_exit(mut t: Transfer) -> Transfer {
    let data = unsafe {
        // Drop the stack
        let stack_ref = &mut *(t.data as *mut Option<(ProtectedFixedSizeStack, usize)>);
        let (_, result) = stack_ref.take().unwrap();
        result
    };

    t.data = data;
    t.context = unsafe { mem::transmute(0usize) };
    t
}

extern "C" fn coroutine_unwind(t: Transfer) -> Transfer {
    // Save the Context in the Coroutine object
    // because the `t` won't be able to be passed to the caller
    let coro = unsafe { &mut *(t.data as *mut Coroutine) };

    coro.context = Some(t.context);

    trace!("Coroutine `{}`: unwinding", coro.debug_name());
    panic::resume_unwind(Box::new(ForceUnwind));
}

/// Coroutine state
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum State {
    /// Suspended state (yield from coroutine inside, ready for resume).
    Suspended,
    /// Running state (executing in callback).
    Running,
    /// Parked state. Similar to `Suspended` state, but `Suspended` is representing that coroutine
    /// will be waken up (resume) by scheduler automatically. Coroutines in `Parked` state should
    /// be waken up manually.
    Parked,
    /// Coroutine is finished and internal data has been destroyed.
    Finished,
    /// Coroutine is panicked inside.
    Panicked,
}

/// Coroutine context representation
#[derive(Debug)]
pub struct Coroutine {
    context: Option<Context>,
    name: Option<String>,
    state: State,
    panicked_error: Option<Box<Any + Send + 'static>>,
}

impl Coroutine {
    /// Spawn a coroutine with `Options`
    #[inline]
    pub fn spawn_opts<F>(f: F, opts: Options) -> Handle
        where F: FnOnce(&mut Coroutine, usize) -> usize + 'static
    {
        Self::spawn_opts_impl(Box::new(f) as Thunk<'static>, opts)
    }

    /// Spawn a coroutine with default options
    #[inline]
    pub fn spawn<F>(f: F) -> Handle
        where F: FnOnce(&mut Coroutine, usize) -> usize + 'static
    {
        Self::spawn_opts_impl(Box::new(f), Options::default())
    }

    fn spawn_opts_impl(f: Thunk<'static>, opts: Options) -> Handle {
        let data = InitData {
            stack: ProtectedFixedSizeStack::new(opts.stack_size).expect("failed to acquire stack"),
            callback: f,
        };

        let context = Context::new(&data.stack, coroutine_entry);

        // Give him the initialization data
        let mut data_opt = Some(data);
        let t = context.resume(&mut data_opt as *mut _ as usize);
        debug_assert!(data_opt.is_none());

        let coro_ref = unsafe { &mut *(t.data as *mut Coroutine) };
        coro_ref.context = Some(t.context);

        if let Some(name) = opts.name {
            coro_ref.set_name(name);
        }

        // Done!
        Handle(coro_ref)
    }

    fn take_context(&mut self) -> Context {
        self.context.take().unwrap()
    }

    /// Gets state of Coroutine
    #[inline]
    pub fn state(&self) -> State {
        self.state
    }

    /// Gets name of Coroutine
    #[inline]
    pub fn name(&self) -> Option<&String> {
        self.name.as_ref()
    }

    /// Set name of Coroutine
    #[inline]
    pub fn set_name(&mut self, name: String) {
        self.name = Some(name);
    }

    /// Name for debugging
    #[inline]
    pub fn debug_name(&self) -> String {
        match self.name {
            Some(ref name) => name.clone(),
            None => format!("{:p}", self),
        }
    }

    #[inline(never)]
    fn inner_yield_with_state(&mut self, state: State, data: usize) -> usize {
        let context = self.take_context();

        trace!("Coroutine `{}`: yielding to {:?}",
               self.debug_name(),
               &context);

        self.state = state;

        let Transfer { context, data } = context.resume(data);

        if unsafe { mem::transmute_copy::<_, usize>(&context) } != 0usize {
            self.context = Some(context);
        }
        data
    }

    #[inline]
    fn yield_with_state(&mut self, state: State, data: usize) -> ::Result<usize> {
        let data = self.inner_yield_with_state(state, data);

        if self.state() == State::Panicked {
            match self.panicked_error.take() {
                Some(err) => Err(::Error::Panicking(err)),
                None => Err(::Error::Panicked),
            }
        } else {
            Ok(data)
        }
    }

    /// Yield the current coroutine with `Suspended` state
    #[inline]
    pub fn yield_with(&mut self, data: usize) -> usize {
        self.inner_yield_with_state(State::Suspended, data)
    }

    /// Yield the current coroutine with `Parked` state
    #[inline]
    pub fn park_with(&mut self, data: usize) -> usize {
        self.inner_yield_with_state(State::Parked, data)
    }

    fn force_unwind(&mut self) {
        trace!("Coroutine `{}`: force unwinding", self.debug_name());

        let ctx = self.take_context();
        let Transfer { context, .. } =
            ctx.resume_ontop(self as *mut Coroutine as usize, coroutine_unwind);
        self.context = Some(context);

        trace!("Coroutine `{}`: force unwound", self.debug_name());
    }
}

/// Handle for a Coroutine
#[derive(Eq, PartialEq)]
pub struct Handle(*mut Coroutine);

impl Handle {
    #[doc(hidden)]
    #[inline]
    pub fn into_raw(self) -> *mut Coroutine {
        let coro = self.0;
        mem::forget(self);
        coro
    }

    #[doc(hidden)]
    #[inline]
    pub unsafe fn from_raw(coro: *mut Coroutine) -> Handle {
        assert!(!coro.is_null());
        Handle(coro)
    }

    /// Check if the Coroutine is already finished
    #[inline]
    pub fn is_finished(&self) -> bool {
        match self.state() {
            State::Finished | State::Panicked => true,
            _ => false,
        }
    }

    #[inline]
    fn yield_with_state(&mut self, state: State, data: usize) -> ::Result<usize> {
        let coro = unsafe { &mut *self.0 };
        coro.yield_with_state(state, data)
    }

    /// Resume the Coroutine
    #[inline]
    pub fn resume(&mut self, data: usize) -> ::Result<usize> {
        assert!(!self.is_finished());
        self.yield_with_state(State::Running, data)
    }

    /// Gets state of Coroutine
    #[inline]
    pub fn state(&self) -> State {
        let coro = unsafe { &*self.0 };
        coro.state()
    }

    /// Gets name of Coroutine
    #[inline]
    pub fn name(&self) -> Option<&String> {
        let coro = unsafe { &*self.0 };
        coro.name()
    }

    /// Set name of Coroutine
    #[inline]
    pub fn set_name(&mut self, name: String) {
        let coro = unsafe { &mut *self.0 };
        coro.set_name(name)
    }

    /// Name for debugging
    #[inline]
    pub fn debug_name(&self) -> String {
        let coro = unsafe { &*self.0 };
        coro.debug_name()
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        trace!("Coroutine `{}`: dropping with {:?}",
               self.debug_name(),
               self.state());

        let coro = unsafe { &mut *self.0 };

        if !self.is_finished() {
            coro.force_unwind()
        }

        coro.inner_yield_with_state(State::Finished, 0);
    }
}

impl fmt::Debug for Handle {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.is_finished() {
            write!(f, "Coroutine(None, Finished)")
        } else {
            write!(f,
                   "Coroutine(Some({}), {:?})",
                   self.debug_name(),
                   self.state())
        }
    }
}

impl Iterator for Handle {
    type Item = ::Result<usize>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.is_finished() {
            None
        } else {
            let x = self.resume(0);
            Some(x)
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn generator() {
        let coro = Coroutine::spawn(|coro, _| {
            for i in 0..10 {
                coro.yield_with(i);
            }
            10
        });

        let ret = coro.map(|x| x.unwrap()).collect::<Vec<usize>>();
        assert_eq!(&ret[..], [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
    }

    #[test]
    fn yield_data() {
        let mut coro = Coroutine::spawn(|coro, data| coro.yield_with(data));

        assert_eq!(coro.resume(0).unwrap(), 0);
        assert_eq!(coro.resume(1).unwrap(), 1);
        assert!(coro.is_finished());
    }

    #[test]
    fn force_unwinding() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct Guard {
            inner: Arc<AtomicUsize>,
        }

        impl Drop for Guard {
            fn drop(&mut self) {
                self.inner.fetch_add(1, Ordering::SeqCst);
            }
        }

        let orig = Arc::new(AtomicUsize::new(0));

        {
            let pass = orig.clone();
            let mut coro = Coroutine::spawn(move |coro, _| {
                let _guard = Guard { inner: pass.clone() };
                coro.yield_with(0);
                let _guard2 = Guard { inner: pass };
                0
            });

            let _ = coro.resume(0);
            // Let it drop
        }

        assert_eq!(orig.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn unwinding() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct Guard {
            inner: Arc<AtomicUsize>,
        }

        impl Drop for Guard {
            fn drop(&mut self) {
                self.inner.fetch_add(1, Ordering::SeqCst);
            }
        }

        let orig = Arc::new(AtomicUsize::new(0));

        {
            let pass = orig.clone();
            let mut coro = Coroutine::spawn(move |_, _| {
                let _guard = Guard { inner: pass.clone() };
                panic!("111");
            });

            let _ = coro.resume(0);
            // Let it drop
        }

        assert_eq!(orig.load(Ordering::SeqCst), 1);
    }

    #[test]
    #[should_panic]
    fn resume_after_finished() {
        let mut coro = Coroutine::spawn(|_, _| 0);
        let _ = coro.resume(0);
        let _ = coro.resume(0);
    }

    #[test]
    fn state() {
        let mut coro = Coroutine::spawn(|coro, _| {
            coro.yield_with(0);
            coro.park_with(0);
            0
        });

        assert_eq!(coro.state(), State::Suspended);
        let _ = coro.resume(0);
        assert_eq!(coro.state(), State::Suspended);
        let _ = coro.resume(0);
        assert_eq!(coro.state(), State::Parked);
        let _ = coro.resume(0);
        assert_eq!(coro.state(), State::Finished);
    }

    #[test]
    fn panicking() {
        let mut coro = Coroutine::spawn(|_, _| {
            panic!(1010);
        });

        let result = coro.resume(0);
        println!("{:?} {:?}", result, coro.state());
        assert!(result.is_err());

        let err = result.unwrap_err();

        match err {
            ::Error::Panicking(err) => {
                assert!(err.is::<i32>());
            }
            _ => unreachable!(),
        }
    }
}
