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

extern crate alloc;

use std::fmt;
use std::env::page_size;
use std::ptr;

/// A task's stack. The name "Stack" is a vestige of segmented stacks.
pub struct Stack {
    buf: Option<*mut u8>,
    len: usize,
    min_size: usize,
}

impl fmt::Debug for Stack {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        try!(write!(f, "Stack {} buf: ", "{"));
        match self.buf {
            Some(ref map) => try!(write!(f, "Some({:?}), ", map)),
            None => try!(write!(f, "None, ")),
        }
        write!(f, "min_size: {:?} {}", self.min_size, "}")
    }
}

impl Drop for Stack {
    fn drop(&mut self) {
        match self.buf {
            Some(ref b) => unsafe {
                alloc::heap::deallocate(*b, self.len, 0x1000);
            },
            None => {}
        }
    }
}

impl Stack {
    /// Allocate a new stack of `size`. If size = 0, this will fail. Use
    /// `dummy_stack` if you want a zero-sized stack.
    pub fn new(size: usize) -> Stack {

        let len = round_up(size, page_size());

        let buf = unsafe {
            // 4K align
            alloc::heap::allocate(len, 0x1000)
        };

        Stack {
            buf: Some(buf),
            len: len,
            min_size: size,
        }
    }

    /// Create a 0-length stack which starts (and ends) at 0.
    #[allow(dead_code)]
    pub unsafe fn dummy_stack() -> Stack {
        Stack {
            buf: None,
            len: 0,
            min_size: 0,
        }
    }

    #[allow(dead_code)]
    pub fn guard(&self) -> *const usize {
        (self.start() as usize + page_size()) as *const usize
    }

    /// Point to the low end of the allocated stack
    pub fn start(&self) -> *const usize {
        self.buf.as_ref()
            .map(|m| *m as *const usize)
            .unwrap_or(ptr::null())
    }

    /// Point one usize beyond the high end of the allocated stack
    pub fn end(&self) -> *const usize {
        self.buf
            .as_ref()
            .map(|buf| unsafe {
                buf.offset(self.len as isize) as *const usize
            })
            .unwrap_or(ptr::null())
    }

    /// Minimun size of this stack
    #[inline(always)]
    pub fn min_size(&self) -> usize {
        self.min_size
    }
}

// Round up `from` to be divisible by `to`
fn round_up(from: usize, to: usize) -> usize {
    let r = if from % to == 0 {
        from
    } else {
        from + to - (from % to)
    };
    if r == 0 {
        to
    } else {
        r
    }
}
