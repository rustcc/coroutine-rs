extern crate coroutine;
extern crate num_cpus;

#[cfg(feature = "enable-clonable-handle")]
fn main() {
    use std::thread;

    use coroutine::coroutine::Coroutine;

    let mut threads = Vec::with_capacity(num_cpus::get());

    let coro =
        Coroutine::spawn(move|| {
            for _ in 0..100000 {
                Coroutine::sched();
            }
            println!("Finished");
        });

    for i in 0..num_cpus::get() {
        let coro = coro.clone();
        let t = thread::Builder::new().name(format!("Thread {}", i)).spawn(move|| coro.join().unwrap()).unwrap();
        threads.push(t);
    }

    for t in threads.into_iter() {
        t.join().unwrap();
    }
}

#[cfg(not(feature = "enable-clonable-handle"))]
fn main() {
    panic!("Require `enable-clonable-handle` feature");
}
