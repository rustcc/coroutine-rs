# coroutine-rs

[![Build Status](https://img.shields.io/travis/rustcc/coroutine-rs.svg)](https://travis-ci.org/rustcc/coroutine-rs) [![crates.io](https://img.shields.io/crates/v/coroutine.svg)](https://crates.io/crates/coroutine) [![crates.io](https://img.shields.io/crates/l/coroutine.svg)](https://crates.io/crates/coroutine)

Coroutine library in Rust

```toml
[dependencies.coroutine]
git = "https://github.com/rustcc/coroutine-rs.git"
```

or

```toml
[dependencies]
coroutine = "*"
```

## Usage

Basic usage of Coroutine

```rust
extern crate coroutine;

use coroutine::{spawn, sched};

fn main() {
    // Spawn a new coroutine
    let coro = spawn(move|| {

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

- [x] Basic single threaded coroutine support

- [x] Clonable coroutine handle

- [ ] Thread-safe: can only resume a coroutine in one thread simultaneously

- [ ] Multithreaded scheduler with work stealing feature

- [ ] Asynchronous I/O supports
