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
    wait_list: UnsafeCell<VecDeque<Handle>>,
}

impl<T> Mutex<T> {
    pub fn new(inner: T) -> Mutex<T> {
        Mutex {
            lock: SpinLock::new(),
            inner: UnsafeCell::new(inner),
            wait_list: UnsafeCell::new(VecDeque::new()),
        }
    }

    pub fn into_inner(self) -> T {
        unsafe {
            self.inner.into_inner()
        }
    }

    pub fn lock<'a>(&'a self) -> LockGuard<'a, T> {
        if !self.lock.try_lock() {
            let current = coroutine::current();
            unsafe {
                let wait_list: &mut VecDeque<Handle> = &mut *self.wait_list.get();
                wait_list.push_back(current);
            }
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
        let front = unsafe {
            let wait_list: &mut VecDeque<Handle> = &mut *self.wait_list.get();
            wait_list.pop_front()
        };

        match front {
            Some(hdl) => {
                coroutine::resume(&hdl).unwrap();
            },
            None => {}
        }
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

    use coroutine::{spawn, sched};

    use super::Mutex;

    #[test]
    fn test_mutex_basic() {
        let lock = Arc::new(Mutex::new(1));

        for _ in 0..10 {
            let lock = lock.clone();
            spawn(move|| {
                let mut guard = lock.lock();

                *guard += 1;
            }).resume();
        }

        assert_eq!(*lock.lock(), 11);
    }
}
