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

use std::mem;

use context::{Context, Stack};

/// Coroutine is nothing more than a context and a stack
#[derive(Debug)]
pub struct Coroutine {
    context: Context,
    stack: Option<Stack>,
}

impl Coroutine {
    /// Spawn an empty coroutine
    #[inline(always)]
    pub unsafe fn empty() -> Coroutine {
        Coroutine {
            context: Context::empty(),
            stack: None,
        }
    }

    /// New a coroutine with initialized context and stack
    #[inline(always)]
    pub fn new(ctx: Context, stack: Stack) -> Coroutine {
        Coroutine {
            context: ctx,
            stack: Some(stack),
        }
    }

    /// Yield to another coroutine
    #[inline(always)]
    pub fn yield_to(&mut self, target: &Coroutine) {
        Context::swap(&mut self.context, &target.context);
    }

    /// Take out the stack
    #[inline(always)]
    pub fn take_stack(mut self) -> Option<Stack> {
        self.stack.take()
    }

    /// Get context
    #[inline(always)]
    pub fn context(&self) -> &Context {
        &self.context
    }

    /// Get mutable context
    #[inline(always)]
    pub fn context_mut(&mut self) -> &mut Context {
        &mut self.context
    }

    /// Set stack
    #[inline(always)]
    pub fn set_stack(&mut self, stack: Stack) -> Option<Stack> {
        let mut opt_stack = Some(stack);
        mem::swap(&mut self.stack, &mut opt_stack);
        opt_stack
    }
}
