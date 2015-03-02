#![feature(std_misc)]

extern crate coroutine;

use std::thread;
use std::rt::util::default_sched_threads;

use coroutine::mpmc_bounded_queue::Queue;
use coroutine::coroutine::{spawn, sched, State};

fn main() {
    const MAX_MESSAGES_NUM: usize = 1000;
    let n_threads = default_sched_threads();

    let queue = Queue::with_capacity(n_threads * MAX_MESSAGES_NUM);

    let coro = spawn(move || {
        let mut count = 0;
        loop {
            println!("Counting {} in {:?}", count, thread::current());
            count += 1;

            sched();
        }
    });

    queue.push(coro);

    let mut threads = Vec::with_capacity(n_threads);
    for thread_id in 0..n_threads {
        let queue = queue.clone();
        let t = thread::Builder::new().name(format!("Thread {}", thread_id)).spawn(move || {
            loop {
                match queue.pop() {
                    Some(mut coro) => {
                        coro = coro.resume().unwrap();

                        match coro.state() {
                            State::Suspended => {
                                queue.push(coro);
                            },
                            _ => {}
                        }
                    },
                    None => {}
                }
            }
        }).unwrap();

        threads.push(t);
    }

    for t in threads.into_iter() {
        t.join().unwrap();
    }
}
