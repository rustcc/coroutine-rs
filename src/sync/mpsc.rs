// The MIT License (MIT)

// Copyright (c) 2015 Rustcc Develpers

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

use std::sync::mpsc;

use coroutine::Coroutine;

#[derive(Clone)]
pub struct Sender<T> {
    inner: mpsc::Sender<T>,
}

pub struct SyncSender<T> {
    inner: mpsc::SyncSender<T>,
}

unsafe impl<T: Send> Send for SyncSender<T> {}

impl<T> !Sync for SyncSender<T> {}

pub struct Receiver<T> {
    inner: mpsc::Receiver<T>,
}

impl<T> Sender<T> {
    fn new(inner: mpsc::Sender<T>) -> Sender<T> {
        Sender {
            inner: inner,
        }
    }

    pub fn send(&self, data: T) -> Result<(), mpsc::SendError<T>> {
        try!(self.inner.send(data));
        Coroutine::sched();
    }

    pub fn try_send(&self, data: T) -> Result<(), mpsc::TrySendError<T>> {

    }
}


