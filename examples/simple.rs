extern crate coroutine;

use std::default::Default;

use coroutine::coroutine::{Coroutine, Options};

fn main() {
    // Spawn a new coroutine
    let mut opts = Options::default();
    opts.name = Some("Outer".to_string());

    let coro = Coroutine::spawn_opts(move|| {

        println!("Hello in coroutine!");

        // Yield back to it's parent
        Coroutine::sched();

        println!("We are back!!");

        // Spawn a new coroutine
        let mut opts = Options::default();
        opts.name = Some("Inner".to_string());
        Coroutine::spawn_opts(move|| {
            println!("Hello inside");
        }, opts).join().unwrap();

        println!("Good bye");
    }, opts).resume().unwrap();

    println!("We are here!");

    // Resume the coroutine
    coro.resume().unwrap();

    println!("Back to main.");
}
