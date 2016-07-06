extern crate coroutine;

use std::usize;
use coroutine::asymmetric::Coroutine;

fn main() {
    let coro = Coroutine::spawn(|me,_| {
        for num in 0..10 {
            me.yield_with(num);
        }
        usize::MAX
    });

    for num in coro {
        println!("{}", num);
    }
}
