extern crate coroutine;
extern crate env_logger;

use std::usize;
use coroutine::asymmetric::Coroutine;

fn main() {
    env_logger::init().unwrap();

    let coro = Coroutine::spawn(|me, _| {
        for num in 0..10 {
            me.yield_with(num);
        }
        usize::MAX
    });

    for num in coro {
        println!("{}", num.unwrap());
    }
}
