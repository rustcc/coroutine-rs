// The MIT License (MIT)

// Copyright (c) 2015 Rustcc Developers

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

use std::collections::VecDeque;
use std::ops::{Deref, DerefMut};
use std::cell::UnsafeCell;

use sync::spinlock::SpinLock;
use coroutine::{self, Coroutine, Handle};

pub struct Mutex<T> {
    lock: SpinLock,
    inner: UnsafeCell<T>,
}

impl<T> Mutex<T> {
    pub fn new(inner: T) -> Mutex<T> {
        Mutex {
            lock: SpinLock::new(),
            inner: UnsafeCell::new(inner),
        }
    }

    pub fn into_inner(self) -> T {
        unsafe {
            self.inner.into_inner()
        }
    }

    pub fn lock<'a>(&'a self) -> LockGuard<'a, T> {
        if !self.lock.try_lock() {
            coroutine::sched();
        }

        LockGuard::new(self, &self.inner)
    }

    pub fn try_lock<'a>(&'a self) -> Option<LockGuard<'a, T>> {
        if self.lock.try_lock() {
            Some(LockGuard::new(self, &self.inner))
        } else {
            None
        }
    }

    fn unlock(&self) {
        self.lock.unlock();
    }
}

unsafe impl<T: Send> Send for Mutex<T> {}

unsafe impl<T: Send> Sync for Mutex<T> {}

pub struct LockGuard<'a, T: 'a> {
    mutex: &'a Mutex<T>,
    data: &'a UnsafeCell<T>,
}

impl<'a, T: 'a> LockGuard<'a, T> {
    fn new(mutex: &'a Mutex<T>, data: &'a UnsafeCell<T>) -> LockGuard<'a, T> {
        LockGuard {
            mutex: mutex,
            data: data,
        }
    }
}

impl<'a, T: 'a> Drop for LockGuard<'a, T> {
    fn drop(&mut self) {
        self.mutex.unlock()
    }
}

impl<'a, T: 'a> Deref for LockGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe {
            &*self.data.get()
        }
    }
}

impl<'a, T: 'a> DerefMut for LockGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe {
            &mut *self.data.get()
        }
    }
}

#[cfg(test)]
mod test {
    use std::sync::Arc;
    use std::thread;

    use coroutine::{spawn, sched};

    use super::Mutex;

    #[test]
    fn test_mutex_basic() {
        let lock = Arc::new(Mutex::new(0));

        let mut futs = Vec::new();

        for _ in 0..10 {
            println!("??");
            let lock = lock.clone();
            let fut = thread::scoped(move|| {
                spawn(move|| {
                    let mut guard = lock.lock();
                    for _ in 0..100_0000 {
                        *guard += 1;
                    }
                    println!("HERE!!");
                }).resume().unwrap();
            });
            futs.push(fut);
        }

        for fut in futs.into_iter() {
            fut.join();
        }

        assert_eq!(*lock.lock(), 100_0000 * 10);
    }
}
