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

use std::sync::atomic::{AtomicBool, Ordering};

/// Spinlock
pub struct SpinLock {
    flag: AtomicBool,
}

impl SpinLock {
    /// Create a new Spinlock
    pub fn new() -> SpinLock {
        SpinLock {
            flag: AtomicBool::new(false),
        }
    }

    pub fn try_lock(&self) -> bool {
        !self.flag.compare_and_swap(false, true, Ordering::Acquire)
    }

    pub fn lock(&self) {
        while !self.try_lock() {}
    }

    pub fn unlock(&self) {
        self.flag.store(false, Ordering::Release)
    }
}

#[cfg(test)]
mod test {
    use super::SpinLock;

    #[test]
    fn test_spinlock_basic() {
        let lock = SpinLock::new();

        assert!(lock.try_lock());

        assert!(!lock.try_lock());

        lock.unlock();
    }
}
