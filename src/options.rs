//! Coroutine options

const DEFAULT_STACK_SIZE: usize = 2 * 1024 * 1024; // 2M

/// Coroutine spawn options
#[derive(Debug)]
pub struct Options {
    /// The size of the stack
    pub stack_size: usize,

    /// The name of the Coroutine
    pub name: Option<String>,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            stack_size: DEFAULT_STACK_SIZE,
            name: None,
        }
    }
}
