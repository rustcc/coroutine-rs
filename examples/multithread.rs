#![feature(std_misc)]

extern crate coroutine;

use std::thread;
use std::rt::util::default_sched_threads;

use coroutine::coroutine::Coroutine;

fn main() {

    let mut threads = Vec::with_capacity(default_sched_threads());

    for i in 0..default_sched_threads() {
        let t = thread::scoped(move|| {
            Coroutine::spawn(move|| {
                let thread_id = i;
                Coroutine::spawn(move|| {
                    for count in 0..10 {
                        println!("Coroutine running in thread {}: counting {}", thread_id, count);
                        Coroutine::sched();
                    }
                }).join().unwrap();
            }).join().unwrap();
        });
        threads.push(t);
    }

    for t in threads.into_iter() {
        t.join();
    }
}
