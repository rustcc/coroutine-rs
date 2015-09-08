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

use asymmetric;
use options::Options;

/// Coroutine configuration. Provides detailed control over the properties and behavior of new Coroutines.
///
/// ```ignore
/// let coro = AsymmetricBuilder::new().name(format!("Coroutine #{}", 1))
///                                    .stack_size(4096)
///                                    .spawn(|_| println!("Hello world!!"));
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
    pub fn spawn_asymmetric<T, F>(self, f: F) -> asymmetric::Handle<T>
        where F: FnOnce(&mut asymmetric::Coroutine<T>) + Send + 'static,
              T: Send + 'static
    {
        asymmetric::Coroutine::spawn_opts(f, self.opts)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_asymmetric_builder_basic() {
        let ret = Builder::new().name("Test builder".to_string()).spawn_asymmetric(move|me| {
            me.yield_with(1);
        }).resume();
        assert_eq!(Some(1), ret.unwrap());
    }
}
