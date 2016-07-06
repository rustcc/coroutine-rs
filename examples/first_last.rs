extern crate coroutine;

use coroutine::asymmetric::Coroutine;

fn main() {
    let mut coro = Coroutine::spawn(|_, initial| {
        println!("Initial value: {}", initial);
        2
    });

    println!("Final value: {}", coro.resume(1));
}
