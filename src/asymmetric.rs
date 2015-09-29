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

extern crate context;
extern crate libc;

use std::iter::Iterator;
use std::mem::transmute;
use std::cell::UnsafeCell;
use std::default::Default;
use std::ops::DerefMut;
use std::fmt;
use std::boxed::FnBox;
use std::thread;

use context::Context;
use context::stack::{Stack, StackPool};

use options::Options;

// Catch panics inside coroutines
unsafe fn try<R, F: FnOnce() -> R>(f: F) -> thread::Result<R> {
    let mut f = Some(f);
    let f = &mut f as *mut Option<F> as usize;
    thread::catch_panic(move || {
        (*(f as *mut Option<F>)).take().unwrap()()
    })
}


thread_local!(static STACK_POOL: UnsafeCell<StackPool> = UnsafeCell::new(StackPool::new()));

struct ForceUnwind;

/// Initialization function for make context
extern "C" fn coroutine_initialize(_: usize, f: *mut libc::c_void) -> ! {
    {
        let func: Box<Box<FnBox()>> = unsafe {
            Box::from_raw(f as *mut Box<FnBox()>)
        };

        func();
    }

    unreachable!("Never reach here");
}

#[derive(Debug, Copy, Clone)]
enum State {
    Created,
    Running,
    Finished,
    ForceUnwind,
}

#[allow(raw_pointer_derive)]
#[derive(Debug)]
struct CoroutineImpl<T = ()>
    where T: Send
{
    parent: Context,
    context: Context,
    stack: Option<Stack>,

    name: Option<String>,
    state: State,

    result: Option<::Result<*mut Option<T>>>,
}

unsafe impl<T> Send for CoroutineImpl<T>
    where T: Send,
{}

impl<T> fmt::Display for CoroutineImpl<T>
    where T: Send
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Coroutine({})", self.name.as_ref()
                                            .map(|s| &s[..])
                                            .unwrap_or("<unnamed>"))
    }
}

impl<T> CoroutineImpl<T>
    where T: Send,
{
    unsafe fn yield_back(&mut self) -> Option<T> {
        Context::swap(&mut self.context, &self.parent);

        if let State::ForceUnwind = self.state {
            panic!(ForceUnwind);
        }

        match self.result.take() {
            None => None,
            Some(Ok(x)) => (*x).take(),
            _ => unreachable!("Coroutine is panicking"),
        }
    }

    unsafe fn resume(&mut self) -> ::Result<Option<T>> {
        Context::swap(&mut self.parent, &self.context);
        match self.result.take() {
            None => Ok(None),
            Some(Ok(x)) => Ok((*x).take()),
            Some(Err(err)) => Err(err),
        }
    }

    pub fn name(&self) -> Option<&str> {
        self.name.as_ref().map(|s| &s[..])
    }

    fn take_data(&mut self) -> Option<T> {
        match self.result.take() {
            None => None,
            Some(Ok(x)) => unsafe { (*x).take() },
            _ => unreachable!("Coroutine is panicking")
        }
    }

    unsafe fn yield_with(&mut self, data: T) -> Option<T> {
        self.result = Some(Ok(&mut Some(data)));
        self.yield_back()
    }

    unsafe fn resume_with(&mut self, data: T) -> ::Result<Option<T>> {
        self.result = Some(Ok(&mut Some(data)));
        self.resume()
    }

    unsafe fn force_unwind(&mut self) {
        if let State::Running = self.state {
            self.state = State::ForceUnwind;
            let _ = try(|| { let _ = self.resume(); });
        }
    }
}

impl<T> Drop for CoroutineImpl<T>
    where T: Send,
{
    fn drop(&mut self) {
        unsafe {
            self.force_unwind();
        }
        STACK_POOL.with(|pool| unsafe {
            if let Some(stack) = self.stack.take() {
                (&mut *pool.get()).give_stack(stack);
            }
        });
    }
}

pub struct Coroutine<T>
    where T: Send + 'static,
{
    coro: UnsafeCell<Box<CoroutineImpl<T>>>,
}

impl<T> fmt::Debug for Coroutine<T>
    where T: Send,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self.coro.get())
    }
}

impl<T> Coroutine<T>
    where T: Send,
{
    #[inline]
    pub fn spawn_opts<F>(f: F, opts: Options) -> Coroutine<T>
        where F: FnOnce(CoroutineRef<T>)
    {
        let mut stack = STACK_POOL.with(|pool| unsafe {
            (&mut *pool.get()).take_stack(opts.stack_size)
        });

        let mut coro = Box::new(CoroutineImpl {
            parent: Context::empty(),
            context: Context::empty(),
            stack: None,
            name: opts.name,
            state: State::Created,
            result: None,
        });

        let coro_ref: &mut CoroutineImpl<T> = unsafe {
            let ptr: *mut CoroutineImpl<T> = coro.deref_mut();
            &mut *ptr
        };

        let puller_ref = CoroutineRef {
            coro: coro_ref
        };

        // Coroutine function wrapper
        // Responsible for calling the function and dealing with panicking
        let wrapper = move|| -> ! {
            let ret = unsafe {
                let puller_ref = puller_ref.clone();
                try(|| {
                    let coro_ref: &mut CoroutineImpl<T> = &mut *puller_ref.coro;
                    coro_ref.state = State::Running;
                    f(puller_ref)
                })
            };

            unsafe {
                let coro_ref: &mut CoroutineImpl<T> = &mut *puller_ref.coro;
                coro_ref.state = State::Finished;
            }

            let is_panicked = match ret {
                Ok(..) => false,
                Err(err) => {
                    if let None = err.downcast_ref::<ForceUnwind>() {
                        {
                            let msg = match err.downcast_ref::<&'static str>() {
                                Some(s) => *s,
                                None => match err.downcast_ref::<String>() {
                                    Some(s) => &s[..],
                                    None => "Box<Any>",
                                }
                            };

                            let name = coro_ref.name().unwrap_or("<unnamed>");
                            error!("Coroutine '{}' panicked at '{}'", name, msg);
                        }

                        coro_ref.result = Some(Err(::Error::Panicking(err)));
                        true
                    } else {
                        false
                    }
                }
            };

            loop {
                if is_panicked {
                    coro_ref.result = Some(Err(::Error::Panicked));
                }

                unsafe {
                    coro_ref.yield_back();
                }
            }
        };

        coro.context.init_with(coroutine_initialize, 0, Box::new(wrapper), &mut stack);
        coro.stack = Some(stack);

        Coroutine {
            coro: UnsafeCell::new(coro)
        }
    }

    #[inline]
    pub fn spawn<F>(f: F) -> Coroutine<T>
        where F: FnOnce(CoroutineRef<T>)
    {
        Coroutine::spawn_opts(f, Default::default())
    }

    #[inline]
    pub fn name(&self) -> Option<&str> {
        unsafe {
            (&*self.coro.get()).name()
        }
    }

    #[inline]
    pub fn resume(&self) -> ::Result<Option<T>> {
        unsafe {
            (&mut *self.coro.get()).resume()
        }
    }

    #[inline]
    pub fn resume_with(&self, data: T) -> ::Result<Option<T>> {
        unsafe {
            (&mut *self.coro.get()).resume_with(data)
        }
    }
}

pub struct CoroutineRef<T>
    where T: Send,
{
    coro: *mut CoroutineImpl<T>,
}

impl<T> Copy for CoroutineRef<T>
    where T: Send,
{}

impl<T> Clone for CoroutineRef<T>
    where T: Send,
{
    fn clone(&self) -> CoroutineRef<T> {
        CoroutineRef {
            coro: self.coro,
        }
    }
}

unsafe impl<T> Send for CoroutineRef<T>
    where T: Send,
{}

impl<T> CoroutineRef<T>
    where T: Send,
{
    #[inline]
    pub fn yield_back(&self) -> Option<T> {
        unsafe {
            let coro: &mut CoroutineImpl<T> = transmute(self.coro);
            coro.yield_back()
        }
    }

    #[inline]
    pub fn yield_with(&self, data: T) -> Option<T> {
        unsafe {
            let coro: &mut CoroutineImpl<T> = transmute(self.coro);
            coro.yield_with(data)
        }
    }

    #[inline]
    pub fn name(&self) -> Option<&str> {
        unsafe {
            (&*self.coro).name()
        }
    }

    #[inline]
    pub fn take_data(&self) -> Option<T> {
        unsafe {
            let coro: &mut CoroutineImpl<T> = transmute(self.coro);
            coro.take_data()
        }
    }
}

impl<T> Iterator for Coroutine<T>
    where T: Send,
{
    type Item = ::Result<T>;

    fn next(&mut self) -> Option<::Result<T>> {
        match self.resume() {
            Ok(r) => r.map(|x| Ok(x)),
            Err(err) => Some(Err(err)),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_asymmetric_basic() {
        let coro = Coroutine::spawn(|me| {
            for i in 0..10 {
                me.yield_with(i);
            }
        });

        for (i, j) in coro.zip(0..10) {
            assert_eq!(i.unwrap(), j);
        }
    }

    #[test]
    fn test_asymmetric_panic() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let counter = Arc::new(AtomicUsize::new(0));

        let cloned_counter = counter.clone();
        let coro: Coroutine<()> = Coroutine::spawn(move|_| {
            struct Foo(Arc<AtomicUsize>);

            impl Drop for Foo {
                fn drop(&mut self) {
                    self.0.store(1, Ordering::SeqCst);
                }
            }

            let _foo = Foo(cloned_counter);

            panic!("Panicked inside");
        });

        let result = coro.resume();
        assert!(result.is_err());

        assert_eq!(1, counter.load(Ordering::SeqCst));
    }

    #[test]
    fn test_asymmetric_unwinding() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let counter = Arc::new(AtomicUsize::new(0));

        {
            let cloned_counter = counter.clone();
            let coro: Coroutine<()> = Coroutine::spawn(move|me| {
                struct Foo(Arc<AtomicUsize>);

                impl Drop for Foo {
                    fn drop(&mut self) {
                        self.0.store(1, Ordering::SeqCst);
                    }
                }

                let _foo = Foo(cloned_counter);

                me.yield_back();

                unreachable!("Never reach here");
            });

            let result = coro.resume();
            assert!(result.is_ok());

            // Destroy the coro without resume
        }

        assert_eq!(1, counter.load(Ordering::SeqCst));
    }
}

#[cfg(test)]
mod bench {
    use super::*;

    use test::Bencher;

    #[bench]
    fn bench_asymmetric_spawn(b: &mut Bencher) {
        b.iter(|| {
            let _coro: Coroutine<i32> = Coroutine::spawn(|_| { 1; });
        });
    }

    #[bench]
    fn bench_asymmetric_resume(b: &mut Bencher) {
        let coro: Coroutine<()> = Coroutine::spawn(|me| {
            while let None = me.yield_back() {}
        });

        b.iter(|| {
            coro.resume().unwrap();
        });

        coro.resume_with(()).unwrap();;
    }
}
