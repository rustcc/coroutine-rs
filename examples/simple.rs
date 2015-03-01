extern crate coroutine;

use coroutine::coroutine::Coroutine;

fn main() {
    // Spawn a new coroutine
    let coro = Coroutine::spawn(move|| {

        println!("Hello in coroutine!");

        // Yield back to it's parent
        Coroutine::sched();

        println!("We are back!!");

        // Spawn a new coroutine
        Coroutine::spawn(move|| {
            println!("Hello inside");
        });

        println!("Good bye");
    });

    println!("We are here!");

    // Resume the coroutine
    Coroutine::resume(&coro);

    println!("Back to main.");
}
