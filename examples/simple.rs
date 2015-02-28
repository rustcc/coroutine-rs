extern crate coroutine;

use coroutine::coroutine::Environment;

fn main() {
    // Initialize a new environment
    let mut env = Environment::new();

    // Spawn a new coroutine
    let mut coro = env.spawn(move|env| {
        // env is the environment that spawn this coroutine

        println!("Hello in coroutine!");

        // Yield back to it's parent
        env.yield_now();

        println!("We are back!!");

        let subcoro = env.spawn(move|_| {
            println!("Hello inside");
        });

        // Destroy the coroutine and try to resue its stack
        // It is optional but recommended
        env.recycle(subcoro);

        println!("Good bye");
    });

    println!("We are here!");

    // Resume the coroutine
    env.resume(&mut coro);

    println!("Back to main.");
    env.recycle(coro);
}
