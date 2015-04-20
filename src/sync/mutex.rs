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

use std::collections::LinkedList;

use sync::spinlock::SpinLock;
use coroutine::{self, Coroutine, Handle};

pub struct Mutex<T> {
    lock: SpinLock,
    inner: T,
    wait_list: LinkedList<Handle>,
}

impl<T> Mutex<T> {
    pub fn new(inner: T) -> Mutex<T> {
        Mutex {
            lock: SpinLock::new(),
            inner: inner,
            wait_list: LinkedList::new(),
        }
    }

    pub fn into_inner(self) -> T {
        self.inner
    }

    pub fn lock<'a>(&'a mut self) -> LockGuard<'a, T> {
        match self.try_lock() {
            Some(lg) => lg,
            None => {


                LockGuard::new(self)
            }
        }
    }

    pub fn try_lock<'a>(&'a self) -> Option<LockGuard<'a, T>> {
        if self.lock.try_lock() {
            Some(LockGuard::new(self))
        } else {
            None
        }
    }

    fn unlock(&self) {

    }
}

pub struct LockGuard<'a, T: 'a> {
    mutex: &'a Mutex<T>,
}

impl<'a, T: 'a> LockGuard<'a, T> {
    fn new(mutex: &'a Mutex<T>) -> LockGuard<'a, T> {
        LockGuard {
            mutex: mutex,
        }
    }
}

impl<'a, T: 'a> Drop for LockGuard<'a, T> {
    fn drop(&mut self) {
        self.mutex.unlock()
    }
}
