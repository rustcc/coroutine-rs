// The MIT License (MIT)

// Copyright (c) 2015 Rustcc Developers

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

use std::iter::Iterator;
use std::mem::transmute;
use std::cell::UnsafeCell;
use std::default::Default;
use std::ops::DerefMut;
use std::fmt;
use std::rt::unwind::try;
use std::rt::unwind::begin_unwind;

use context::stack::StackPool;
use context::thunk::Thunk;

use options::Options;

use coroutine::raw;

use Result;

thread_local!(static STACK_POOL: UnsafeCell<StackPool> = UnsafeCell::new(StackPool::new()));

struct ForceUnwind;

/// Initialization function for make context
extern "C" fn coroutine_initialize(_: usize, f: *mut ()) -> ! {
    let func: Box<Thunk> = unsafe { transmute(f) };
    func.invoke(());
    loop {}
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
struct CoroutineImpl<F, T = ()>
    where T: Send,
          F: FnMut(CoroutineRef<F, T>)
{
    parent: raw::Coroutine,
    raw_impl: Option<raw::Coroutine>,

    name: Option<String>,
    function: F,
    state: State,

    result: Option<Result<*mut Option<T>>>,
}

unsafe impl<F, T> Send for CoroutineImpl<F, T>
    where T: Send,
          F: FnMut(CoroutineRef<F, T>)
{}

impl<F, T> fmt::Display for CoroutineImpl<F, T>
    where T: Send,
          F: FnMut(CoroutineRef<F, T>)
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Coroutine({})", self.name.as_ref()
                                            .map(|s| &s[..])
                                            .unwrap_or("<unnamed>"))
    }
}

impl<F, T> CoroutineImpl<F, T>
    where T: Send,
          F: FnMut(CoroutineRef<F, T>)
{
    unsafe fn yield_back(&mut self) -> Option<T> {
        self.raw_impl.as_mut().unwrap().yield_to(&self.parent);

        if let State::ForceUnwind = self.state {
            // Begin unwind to force memory reclain, no body cares what the file and line is
            begin_unwind(ForceUnwind, &(file!(), line!()));
        }

        match self.result.take() {
            None => None,
            Some(Ok(x)) => (*x).take(),
            _ => unreachable!("Coroutine is panicking"),
        }
    }

    unsafe fn resume(&mut self) -> Result<Option<T>> {
        self.parent.yield_to(&self.raw_impl.as_ref().unwrap());
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

    unsafe fn resume_with(&mut self, data: T) -> Result<Option<T>> {
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

impl<F, T> Drop for CoroutineImpl<F, T>
    where T: Send,
          F: FnMut(CoroutineRef<F, T>)
{
    fn drop(&mut self) {
        unsafe {
            self.force_unwind();
        }
        STACK_POOL.with(|pool| unsafe {
            if let Some(stack) = self.raw_impl.take().unwrap().take_stack() {
                (&mut *pool.get()).give_stack(stack);
            }
        });
    }
}

pub struct Coroutine<F, T>
    where T: Send + 'static,
          F: FnMut(CoroutineRef<F, T>)
{
    coro: UnsafeCell<Box<CoroutineImpl<F, T>>>,
}

impl<F, T> fmt::Debug for Coroutine<F, T>
    where T: Send,
          F: FnMut(CoroutineRef<F, T>)
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self.coro.get())
    }
}

impl<F, T> Coroutine<F, T>
    where T: Send,
          F: FnMut(CoroutineRef<F, T>)
{
    #[inline]
    pub fn spawn_opts(f: F, opts: Options) -> Coroutine<F, T> {
        let mut stack = STACK_POOL.with(|pool| unsafe {
            (&mut *pool.get()).take_stack(opts.stack_size)
        });

        let mut coro = Box::new(CoroutineImpl {
            parent: unsafe { raw::Coroutine::empty() },
            raw_impl: unsafe { Some(raw::Coroutine::empty()) },
            name: opts.name,
            function: f,
            state: State::Created,
            result: None,
        });

        let coro_ref: &mut CoroutineImpl<F, T> = unsafe {
            let ptr: *mut CoroutineImpl<F, T> = coro.deref_mut();
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
                    let coro_ref: &mut CoroutineImpl<F, T> = &mut *puller_ref.coro;
                    coro_ref.state = State::Running;
                    (coro_ref.function)(puller_ref)
                })
            };

            unsafe {
                let coro_ref: &mut CoroutineImpl<F, T> = &mut *puller_ref.coro;
                coro_ref.state = State::Finished;
            }

            let is_panicked = match ret {
                Ok(..) => false,
                Err(err) => {
                    if let None = err.downcast_ref::<ForceUnwind>() {
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

                            let name = coro_ref.name().unwrap_or("<unnamed>");
                            let _ = writeln!(&mut stderr(), "Coroutine '{}' panicked at '{}'", name, msg);
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

        {
            let coro_ref = coro.raw_impl.as_mut().unwrap();
            coro_ref.context_mut().init_with(coroutine_initialize, 0, wrapper, &mut stack);
            coro_ref.set_stack(stack);
        }

        Coroutine {
            coro: UnsafeCell::new(coro)
        }
    }

    #[inline]
    pub fn spawn(f: F) -> Coroutine<F, T> {
        Coroutine::spawn_opts(f, Default::default())
    }

    #[inline]
    pub fn name(&self) -> Option<&str> {
        unsafe {
            (&*self.coro.get()).name()
        }
    }

    #[inline]
    pub fn resume(&self) -> Result<Option<T>> {
        unsafe {
            (&mut *self.coro.get()).resume()
        }
    }

    #[inline]
    pub fn resume_with(&self, data: T) -> Result<Option<T>> {
        unsafe {
            (&mut *self.coro.get()).resume_with(data)
        }
    }
}

pub struct CoroutineRef<F, T>
    where T: Send,
          F: FnMut(CoroutineRef<F, T>)
{
    coro: *mut CoroutineImpl<F, T>,
}

impl<F, T> Copy for CoroutineRef<F, T>
    where T: Send,
          F: FnMut(CoroutineRef<F, T>)
{}

impl<F, T> Clone for CoroutineRef<F, T>
    where T: Send,
          F: FnMut(CoroutineRef<F, T>)
{
    fn clone(&self) -> CoroutineRef<F, T> {
        CoroutineRef {
            coro: self.coro,
        }
    }
}

unsafe impl<F, T> Send for CoroutineRef<F, T>
    where T: Send,
          F: FnMut(CoroutineRef<F, T>)
{}

impl<F, T> CoroutineRef<F, T>
    where T: Send,
          F: FnMut(CoroutineRef<F, T>)
{
    #[inline]
    pub fn yield_back(&self) -> Option<T> {
        unsafe {
            let coro: &mut CoroutineImpl<F, T> = transmute(self.coro);
            coro.yield_back()
        }
    }

    #[inline]
    pub fn yield_with(&self, data: T) -> Option<T> {
        unsafe {
            let coro: &mut CoroutineImpl<F, T> = transmute(self.coro);
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
            let coro: &mut CoroutineImpl<F, T> = transmute(self.coro);
            coro.take_data()
        }
    }
}

impl<F, T> Iterator for Coroutine<F, T>
    where T: Send,
          F: FnMut(CoroutineRef<F, T>)
{
    type Item = Result<T>;

    fn next(&mut self) -> Option<Result<T>> {
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
    fn test_coroutine_basic() {
        let coro = Coroutine::spawn(|me| {
            for num in 0..10 {
                me.yield_with(num);
            }
        });

        for num in 0..10 {
            assert_eq!(Some(num), coro.resume().unwrap());
        }
    }

    #[test]
    fn test_coroutine_generator() {
        let generator = Coroutine::spawn(|me| {
            for num in 0..10 {
                me.yield_with(num);
            }
        });

        for (actual, num) in generator.enumerate() {
            assert_eq!(actual, num.unwrap());
        }
    }

    #[test]
    fn test_panic_inside() {
        let will_panic: Coroutine<_, ()> = Coroutine::spawn(|_| {
            panic!("Panic inside");
        });

        assert!(will_panic.resume().is_err());
    }

    #[test]
    fn test_coroutine_push() {
        let coro = Coroutine::spawn(|me| {
            assert_eq!(Some(0), me.take_data());

            for num in 1..10 {
                assert_eq!(Some(num), me.yield_back());
            }
        });

        for num in 0..10 {
            coro.resume_with(num).unwrap();
        }
    }

    #[test]
    fn test_coroutine_pull_struct() {
        #[derive(Eq, PartialEq, Debug)]
        struct Foo(i32);

        let coro = Coroutine::spawn(|me| {
            for num in 0..10 {
                me.yield_with(Foo(num));
            }
        });

        for num in 0..10 {
            assert_eq!(Some(Foo(num)), coro.resume().unwrap());
        }
    }

    #[test]
    fn test_coroutine_mut() {
        let mut outer = 0;

        {
            let coro = Coroutine::spawn(|me| {
                loop {
                    outer += 1;
                    me.yield_with(outer);
                }
            });

            for _ in 0..10 {
                coro.resume().unwrap();
            }
        }

        assert_eq!(outer, 10);
    }

    #[test]
    fn test_coroutine_unwind() {
        let mut counter = 0;

        {
            let coro: Coroutine<_, ()> = Coroutine::spawn(|me| {
                struct Guard<'a>(&'a mut i32);

                impl<'a> Drop for Guard<'a> {
                    fn drop(&mut self) {
                        *self.0 += 1;
                    }
                }

                let _guard = Guard(&mut counter);
                *_guard.0 += 1;
                me.yield_back();
            });

            coro.resume().unwrap();
        }

        assert_eq!(counter, 2);
    }
}

#[cfg(test)]
mod benchmark {
    use super::*;

    use test::Bencher;

    #[bench]
    fn bench_coroutine_spawn(b: &mut Bencher) {
        b.iter(|| {
            let _: Coroutine<_, ()> = Coroutine::spawn(move|_| {});
        });
    }

    #[bench]
    fn bench_context_switch(b: &mut Bencher) {
        let coro: Coroutine<_, ()> = Coroutine::spawn(|me| loop { me.yield_back(); });

        b.iter(|| coro.resume());
    }
}
