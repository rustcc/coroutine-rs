#![feature(scoped)]
extern crate coroutine;
extern crate num_cpus;

use std::thread;

use coroutine::coroutine::{State, Coroutine};

fn main() {

    let mut threads = Vec::with_capacity(num_cpus::get());

    let coro =
        Coroutine::spawn(move|| {
            for count in 0..100 {
                println!("Coroutine running in thread {:?}: counting {}", thread::current(), count);
                Coroutine::sched();
                thread::yield_now();
            }
        });

    for i in 0..num_cpus::get() {
        let coro = coro.clone();
        let t = thread::Builder::new().name(format!("Thread {}", i)).scoped(move|| {
            loop {
                match coro.state() {
                    State::Finished | State::Panicked => break,
                    _ => {
                        coro.resume().ok().expect("Failed to resume");
                    },
                }
            }
        }).unwrap();
        threads.push(t);
    }

    for t in threads.into_iter() {
        t.join();
    }
}
