// The MIT License (MIT)

// Copyright (c) 2015 Rustcc developers

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

use std::cell::UnsafeCell;
use std::any::Any;
use std::mem;

use context::stack::{StackPool, Stack};

use coroutine::Coroutine;
use {State, Handle};

thread_local!(static COROUTINE_ENVIRONMENT: UnsafeCell<Box<Environment>> = UnsafeCell::new(Environment::new()));

/// Coroutine managing environment
#[allow(raw_pointer_derive)]
pub struct Environment {
    stack_pool: StackPool,

    coroutine_stack: Vec<*mut Handle>,
    _main_coroutine: Handle,

    running_state: Option<Box<Any + Send>>,

    #[cfg(feature = "enable-clonable-handle")]
    switch_state: State,
}

impl Environment {
    /// Initialize a new environment
    #[cfg(feature = "enable-clonable-handle")]
    fn new() -> Box<Environment> {
        let coro = unsafe {
            let coro = Coroutine::empty(Some("<Environment Root Coroutine>".to_string()), State::Running);
            coro
        };

        let mut env = Box::new(Environment {
            stack_pool: StackPool::new(),

            coroutine_stack: Vec::new(),
            _main_coroutine: coro,

            running_state: None,
            switch_state: State::Suspended,
        });

        let coro: *mut Handle = &mut env._main_coroutine;
        env.coroutine_stack.push(coro);

        env
    }

    /// Initialize a new environment
    #[cfg(not(feature = "enable-clonable-handle"))]
    fn new() -> Box<Environment> {
        let coro = unsafe {
            let coro = Coroutine::empty(Some("<Environment Root Coroutine>".to_string()), State::Running);
            coro
        };

        let mut env = Box::new(Environment {
            stack_pool: StackPool::new(),

            coroutine_stack: Vec::new(),
            _main_coroutine: coro,

            running_state: None,
        });

        let coro: *mut Handle = &mut env._main_coroutine;
        env.coroutine_stack.push(coro);

        env
    }

    #[inline]
    pub fn current() -> &'static mut Environment {
        COROUTINE_ENVIRONMENT.with(|env| unsafe {
            &mut *env.get()
        })
    }

    #[inline]
    pub fn running_count(&self) -> usize {
        self.coroutine_stack.len() - 1
    }

    #[inline]
    pub fn running<'a>(&'a self) -> &'static Handle {
        self.coroutine_stack.last().map(|hdl| unsafe { (& **hdl) })
            .expect("Impossible happened! No current coroutine!")
    }

    #[inline]
    pub fn push(&mut self, hdl: &Handle) {
        self.coroutine_stack.push(unsafe { mem::transmute(hdl) });
    }

    #[inline]
    pub fn pop(&mut self) -> Option<&'static Handle> {
        if self.running_count() == 0 {
            None
        } else {
            self.coroutine_stack.pop().map(|hdl| unsafe { (& *hdl) })
        }
    }

    #[inline]
    pub fn set_resume_result(&mut self, result: Option<Box<Any + Send>>) {
        self.running_state = result;
    }

    #[inline]
    #[cfg(feature = "enable-clonable-handle")]
    pub fn set_switch_state(&mut self, state: State) {
        self.switch_state = state;
    }

    #[inline]
    pub fn take_last_resume_result(&mut self) -> Option<Box<Any + Send>> {
        self.running_state.take()
    }

    #[inline]
    #[cfg(feature = "enable-clonable-handle")]
    pub fn last_switch_state(&mut self) -> State {
        self.switch_state
    }

    #[inline]
    pub fn take_stack(&mut self, size: usize) -> Stack {
        self.stack_pool.take_stack(size)
    }

    #[inline]
    pub fn give_stack(&mut self, stack: Stack) {
        self.stack_pool.give_stack(stack)
    }
}
