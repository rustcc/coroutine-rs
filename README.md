# coroutine-rs

[![Build Status](https://img.shields.io/travis/rustcc/coroutine-rs.svg)](https://travis-ci.org/rustcc/coroutine-rs) [![crates.io](https://img.shields.io/crates/v/coroutine.svg)](https://crates.io/crates/coroutine) [![crates.io](https://img.shields.io/crates/l/coroutine.svg)](https://crates.io/crates/coroutine)

Coroutine library in Rust

```toml
[dependencies.coroutine]
git = "0.5.0"
```

## Usage

Basic usage of Coroutine

```rust
extern crate coroutine;

use std::usize;
use coroutine::asymmetric::Coroutine;

fn main() {
    let coro: Coroutine<i32> = Coroutine::spawn(|me,_| {
        for num in 0..10 {
            me.yield_with(num);
        }
        usize::MAX
    });

    for num in coro {
        println!("{}", num.unwrap());
    }
}
```

This program will print the following to the console

```
0
1
2
3
4
5
6
7
8
9
18446744073709551615
```

For more detail, please run `cargo doc --open`.

## Goals

- [x] Basic single threaded coroutine support

- [x] Asymmetric Coroutines

- [ ] Symmetric Coroutines

- [ ] Thread-safe: can only resume a coroutine in one thread simultaneously

## Notes

* Currently this crate **can only** be built with Rust nightly because of some unstable features.

* Basically it supports arm, i686, mips, mipsel and x86_64 platforms, but we have only tested in

    - OS X 10.10.*, x86_64, nightly

    - ArchLinux, x86_64, nightly

## Thanks

- The Rust developers (context switch ASM from libgreen)
