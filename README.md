# coroutine-rs

Coroutine library in Rust

```toml
[dependencies.coroutine-rs]
git = "https://github.com/rustcc/coroutine-rs.git"
```

## Usage

Basic usage of Coroutine

```rust
extern crate coroutine;

use coroutine::coroutine::{spawn, sched};

fn main() {
    // Spawn a new coroutine
    let mut coro = spawn(move|| {

        println!("Hello in coroutine!");

        // Yield back to it's parent
        sched();

        println!("We are back!!");

        // Spawn a new coroutine
        spawn(move|| {
            println!("Hello inside");
        }).join().unwrap();

        println!("Good bye");
    });

    coro.resume().unwrap();

    println!("We are here!");

    // Resume the coroutine
    coro.resume().unwrap();

    println!("Back to main.");
}
```

## Goals

* Basic single threaded coroutine support

* Multithreaded scheduler with work stealing feature

* I/O supports
