extern crate coroutine;

use coroutine::asymmetric::Coroutine;

fn main() {
    let coro: Coroutine<i32> = Coroutine::spawn(|me| {
        for num in 0..10 {
            me.yield_with(num);
        }
    });

    for num in coro {
        println!("{}", num.unwrap());
    }
}
