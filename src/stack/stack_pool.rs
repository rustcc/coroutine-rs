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

use std::env;
use std::sync::atomic;

pub use stack::Stack;

#[derive(Debug)]
pub struct StackPool {
    // Ideally this would be some data structure that preserved ordering on
    // Stack.min_size.
    stacks: Vec<Stack>,
}

impl StackPool {
    pub fn new() -> StackPool {
        StackPool {
            stacks: vec![],
        }
    }

    pub fn take_stack(&mut self, min_size: usize) -> Stack {
        // Ideally this would be a binary search
        match self.stacks.iter().position(|s| min_size <= s.min_size()) {
            Some(idx) => self.stacks.swap_remove(idx),
            None => Stack::new(min_size)
        }
    }

    pub fn give_stack(&mut self, stack: Stack) {
        if self.stacks.len() <= max_cached_stacks() {
            self.stacks.push(stack)
        }
    }
}

fn max_cached_stacks() -> usize {
    static mut AMT: atomic::AtomicUsize = atomic::ATOMIC_USIZE_INIT;
    match unsafe { AMT.load(atomic::Ordering::SeqCst) } {
        0 => {}
        n => return n - 1,
    }
    let amt = env::var("RUST_MAX_CACHED_STACKS").ok().and_then(|s| s.parse().ok());
    // This default corresponds to 20M of cache per scheduler (at the
    // default size).
    let amt = amt.unwrap_or(10);
    // 0 is our sentinel value, so ensure that we'll never see 0 after
    // initialization has run
    unsafe { AMT.store(amt + 1, atomic::Ordering::SeqCst); }
    return amt;
}

#[cfg(test)]
mod tests {
    use super::StackPool;

    #[test]
    fn stack_pool_caches() {
        let mut p = StackPool::new();
        let s = p.take_stack(10);
        p.give_stack(s);
        let s = p.take_stack(4);
        assert_eq!(s.min_size(), 10);
        p.give_stack(s);
        let s = p.take_stack(14);
        assert_eq!(s.min_size(), 14);
        p.give_stack(s);
    }

    #[test]
    fn stack_pool_caches_exact() {
        let mut p = StackPool::new();
        let s = p.take_stack(10);
        p.give_stack(s);

        let s = p.take_stack(10);
        assert_eq!(s.min_size(), 10);
    }
}
