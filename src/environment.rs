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

use stack::StackPool;
use coroutine::Coroutine;
use {State, Handle};

thread_local!(static COROUTINE_ENVIRONMENT: UnsafeCell<Box<Environment>> = UnsafeCell::new(Environment::new()));

/// Coroutine managing environment
#[allow(raw_pointer_derive)]
pub struct Environment {
    pub stack_pool: StackPool,

    pub coroutine_stack: Vec<*mut Handle>,
    _main_coroutine: Handle,

    pub running_state: Option<Box<Any + Send>>,
    pub switch_state: State,
}

impl Environment {
    /// Initialize a new environment
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

    pub fn current() -> &'static mut Environment {
        COROUTINE_ENVIRONMENT.with(|env| unsafe {
            &mut *env.get()
        })
    }
}
