extern crate coroutine;

use coroutine::Coroutine;

fn main() {
    // Spawn a new coroutine
    let coro = Coroutine::spawn(move|| {

        println!("1. Hello in coroutine!");

        // Yield back to it's parent
        Coroutine::sched();

        println!("3. We are back!!");

        // Spawn a new coroutine
        Coroutine::spawn(move|| {
            println!("4. Begin counting ...");
            for i in 0..5 {
                println!("Counting {}", i);
            }
            println!("5. Counting finished");
        }).join().unwrap();

        println!("6. Good bye");
    });

    // Resume `coro`
    coro.resume().unwrap();;

    println!("2. We are here!");

    // Resume the coroutine
    coro.resume().unwrap();

    println!("7. Back to main.");
}
