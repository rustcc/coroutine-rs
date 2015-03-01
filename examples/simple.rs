extern crate coroutine;

use coroutine::coroutine::{spawn, sched};

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
    }).resume().unwrap();

    println!("We are here!");

    // Resume the coroutine
    coro.resume().unwrap();

    println!("Back to main.");
}
