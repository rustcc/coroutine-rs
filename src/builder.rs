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

use coroutine::{Coroutine, Handle, Options};

/// Coroutine configuration. Provides detailed control over the properties and behavior of new Coroutines.
///
/// ```ignore
/// let coro = Builder::new().name(format!("Coroutine #{}", 1))
///                          .stack_size(4096)
///                          .spawn(|| println!("Hello world!!"));
///
/// coro.resume().unwrap();
/// ```
pub struct Builder {
    opts: Options,
}

impl Builder {
    /// Generate the base configuration for spawning a Coroutine, from which configuration methods can be chained.
    pub fn new() -> Builder {
        Builder {
            opts: Default::default(),
        }
    }

    /// Name the Coroutine-to-be. Currently the name is used for identification only in panic messages.
    pub fn name(mut self, name: String) -> Builder {
        self.opts.name = Some(name);
        self
    }

    /// Set the size of the stack for the new Coroutine.
    pub fn stack_size(mut self, size: usize) -> Builder {
        self.opts.stack_size = size;
        self
    }

    /// Spawn a new Coroutine, and return a handle for it.
    pub fn spawn<F>(self, f: F) -> Handle
        where F: FnOnce() + Send + 'static
    {
        Coroutine::spawn_opts(f, self.opts)
    }
}

#[cfg(test)]
mod test {
    use std::sync::mpsc::channel;

    use super::Builder;

    #[test]
    fn test_builder_basic() {
        let (tx, rx) = channel();
        Builder::new().name("Test builder".to_string()).spawn(move|| {
            tx.send(1).unwrap();
        }).join().ok().expect("Failed to join");
        assert_eq!(rx.recv().unwrap(), 1);
    }
}
