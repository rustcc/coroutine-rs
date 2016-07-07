extern crate coroutine;

use std::usize;
use std::rc::Rc;
use std::cell::RefCell;
use coroutine::asymmetric::Coroutine;

fn main() {

    let rc = Rc::new(RefCell::new(0));

    let rc1 = rc.clone();
    let mut coro1 = Coroutine::spawn(move |me,_| {
        *rc1.borrow_mut() = 1;
        let val = *rc1.borrow();
        me.yield_with(val); // (*rc1.borrow()) - fails with already borrowed
        usize::MAX
    });

    let rc2 = rc.clone();
    let mut coro2 = Coroutine::spawn(move |me,_| {
        *rc2.borrow_mut() = 2;
        let val = *rc2.borrow();
        me.yield_with(val);
        usize::MAX
    });

    println!("First: {}", (*coro1).yield_with(0));
    println!("Second: {}", (*coro2).yield_with(0));
}
