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

use std::boxed::FnBox;
use std::fmt;
use std::usize;
use std::panic;
use std::ptr;
use std::mem;
use std::ops::{Deref, DerefMut};
use std::iter::Iterator;

use context::{Context, Transfer};
use context::stack::ProtectedFixedSizeStack;

use options::Options;

#[derive(Debug)]
struct ForceUnwind;

struct InitData {
    stack: ProtectedFixedSizeStack,
    callback: Box<FnBox(&mut Coroutine, usize)>,
}

extern "C" fn coroutine_entry(t: Transfer) -> ! {
    // Take over the data from Coroutine::spawn_opts
    let InitData { stack, callback } = unsafe {
        let data_opt_ref = &mut *(t.data as *mut Option<InitData>);
        data_opt_ref.take().expect("failed to acquire InitData")
    };

    // This block will ensure the `meta` will be destroied before dropping the stack
    let ctx = {
        let mut meta = Coroutine {
            context: None,
            name: None,
            state: State::Suspended,
        };

        // Yield back after take out the callback function
        // Now the Coroutine is initialized
        let meta_ptr = &mut meta as *mut _ as usize;
        let _ = unsafe {
            ::try(move || {
                let Transfer { context, data } = t.context.resume(meta_ptr);
                let meta_ref = &mut *(meta_ptr as *mut Coroutine);
                meta_ref.context = Some(context);

                // Take out the callback and run it
                callback.call_box((meta_ref, data));

                trace!("Coroutine `{}`: returned from callback", meta_ref.debug_name());
            })
        };

        trace!("Coroutine `{}`: finished => dropping stack", meta.debug_name());

        // If panicked inside, the meta.context stores the actual return Context
        meta.take_context()
    };

    // Drop the stack after it is finished
    let mut stack_opt = Some(stack);
    ctx.resume_ontop(&mut stack_opt as *mut _ as usize, coroutine_exit);

    unreachable!();
}

extern "C" fn coroutine_exit(mut t: Transfer) -> Transfer {
    unsafe {
        // Drop the stack
        let stack_ref = &mut *(t.data as *mut Option<ProtectedFixedSizeStack>);
        let _ = stack_ref.take();
    }

    t.data = usize::MAX;
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

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum State {
    Suspended,
    Running,
    Parked,
}

#[derive(Debug)]
pub struct Coroutine {
    context: Option<Context>,
    name: Option<String>,
    state: State,
}

impl Coroutine {
    #[inline]
    pub fn spawn_opts<F>(f: F, opts: Options) -> Handle
        where F: FnOnce(&mut Coroutine, usize) + 'static
    {
        Self::spawn_opts_impl(Box::new(f), opts)
    }

    #[inline]
    pub fn spawn<F>(f: F) -> Handle
        where F: FnOnce(&mut Coroutine, usize) + 'static
    {
        Self::spawn_opts_impl(Box::new(f), Options::default())
    }

    fn spawn_opts_impl(f: Box<FnBox(&mut Coroutine, usize)>, opts: Options) -> Handle {
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

    pub fn take_context(&mut self) -> Context {
        self.context.take().unwrap()
    }

    #[inline]
    pub fn state(&self) -> State {
        self.state
    }

    #[inline]
    pub fn name(&self) -> Option<&String> {
        self.name.as_ref()
    }

    #[inline]
    pub fn set_name(&mut self, name: String) {
        self.name = Some(name);
    }

    #[inline]
    pub fn debug_name(&self) -> String {
        match self.name {
            Some(ref name) => name.clone(),
            None => format!("{:p}", self),
        }
    }

    #[inline]
    pub fn yield_with(&mut self, data: usize) -> usize {
        let context = self.take_context();

        trace!("Coroutine `{}`: yielding to {:?}",
               self.debug_name(),
               &context);

        self.state = State::Parked;

        let Transfer { context, data } = context.resume(data);
        self.context = Some(context);
        data
    }
}

/// Handle for a Coroutine
#[derive(Eq, PartialEq)]
pub struct Handle(*mut Coroutine);

impl Handle {
    pub unsafe fn empty() -> Handle {
        Handle(ptr::null_mut())
    }

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
        Handle(coro)
    }

    /// Check if the Coroutine is already finished
    #[doc(hidden)]
    #[inline]
    pub fn is_finished(&self) -> bool {
        self.0 == ptr::null_mut()
    }

    /// Resume the Coroutine
    #[doc(hidden)]
    #[inline]
    pub fn resume(&mut self, data: usize) -> usize {
        self.yield_with(State::Running, data)
    }

    /// Yields the Coroutine to the Processor
    #[inline(never)]
    pub fn yield_with(&mut self, state: State, data: usize) -> usize {
        assert!(self.0 != ptr::null_mut());

        let coro = unsafe { &mut *self.0 };
        let context = coro.take_context();

        trace!("Coroutine `{}`: yielding to {:?}",
               coro.debug_name(),
               &context);

        coro.state = state;

        let Transfer { context, data } = context.resume(data);

        // We've returned from a yield to the Processor, because it resume()d us!
        // `context` is the Context of the Processor which we store so we can yield back to it.
        let x: usize = unsafe { mem::transmute_copy(&context) };
        if x == 0usize {
            self.0 = ptr::null_mut();
        } else {
            coro.context = Some(context);
        }
        data
    }
}

impl Deref for Handle {
    type Target = Coroutine;

    #[inline]
    fn deref(&self) -> &Coroutine {
        unsafe { &*self.0 }
    }
}

impl DerefMut for Handle {
    #[inline]
    fn deref_mut(&mut self) -> &mut Coroutine {
        unsafe { &mut *self.0 }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        if self.is_finished() {
            return;
        }

        trace!("Coroutine `{}`: force unwinding", self.debug_name());

        let ctx = self.take_context();
        let coro = mem::replace(&mut self.0, ptr::null_mut());

        ctx.resume_ontop(coro as usize, coroutine_unwind);

        trace!("Coroutine `{}`: force unwound", self.debug_name());
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
    type Item = usize;
    fn next(&mut self) -> Option<Self::Item> {
        if self.is_finished() {
            None
        } else {
            let x = self.resume(0);
            if self.is_finished() {
                None
            } else {
                Some(x)
            }
        }
    }
}
