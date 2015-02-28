// Copyright 2013-2014 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use stack::Stack;
use std::usize;
use std::mem::transmute;
#[cfg(target_arch = "x86_64")]
use std::simd;
use std::thunk::Thunk;

use libc;

use sys;

// FIXME #7761: Registers is boxed so that it is 16-byte aligned, for storing
// SSE regs.  It would be marginally better not to do this. In C++ we
// use an attribute on a struct.
// FIXME #7761: It would be nice to define regs as `Box<Option<Registers>>`
// since the registers are sometimes empty, but the discriminant would
// then misalign the regs again.
#[derive(Debug)]
pub struct Context {
    /// Hold the registers while the task or scheduler is suspended
    regs: Box<Registers>,
    /// Lower bound and upper bound for the stack
    stack_bounds: Option<(usize, usize)>,
}

pub type InitFn = extern "C" fn(usize, *mut ()) -> !; // first argument is task handle, second is thunk ptr

impl Context {
    pub fn empty() -> Context {
        Context {
            regs: box Registers::new(),
            stack_bounds: None,
        }
    }

    /// Create a new context that will resume execution by running start
    ///
    /// The `init` function will be run with `arg` and the `start` procedure
    /// split up into code and env pointers. It is required that the `init`
    /// function never return.
    ///
    /// FIXME: this is basically an awful the interface. The main reason for
    ///        this is to reduce the number of allocations made when a green
    ///        task is spawned as much as possible
    pub fn new<F, A>(init: InitFn, arg: usize, start: F, stack: &mut Stack) -> Context
            where F: FnOnce(A) + Send + 'static {
        let sp: *const usize = stack.end();
        let sp: *mut usize = sp as *mut usize;
        // Save and then immediately load the current context,
        // which we will then modify to call the given function when restored
        let mut regs = box Registers::new();

        initialize_call_frame(&mut regs, init, arg, unsafe { transmute(Box::new(Thunk::with_arg(start))) }, sp);

        // Scheduler tasks don't have a stack in the "we allocated it" sense,
        // but rather they run on pthreads stacks. We have complete control over
        // them in terms of the code running on them (and hopefully they don't
        // overflow). Additionally, their coroutine stacks are listed as being
        // zero-length, so that's how we detect what's what here.
        let stack_base: *const usize = stack.start();
        let bounds =
            if sp as libc::uintptr_t == stack_base as libc::uintptr_t {
                None
            } else {
                Some((stack_base as usize, sp as usize))
            };
        return Context {
            regs: regs,
            stack_bounds: bounds,
        }
    }

    /* Switch contexts

    Suspend the current execution context and resume another by
    saving the registers values of the executing thread to a Context
    then loading the registers from a previously saved Context.
    */
    pub fn swap(out_context: &mut Context, in_context: &Context) {
        debug!("swapping contexts");
        let out_regs: &mut Registers = match out_context {
            &mut Context { regs: ref mut r, .. } => r
        };
        let in_regs: &Registers = match in_context {
            &Context { regs: ref r, .. } => r
        };

        debug!("noting the stack limit and doing raw swap");

        unsafe {
            // Right before we switch to the new context, set the new context's
            // stack limit in the OS-specified TLS slot. This also  means that
            // we cannot call any more rust functions after record_stack_bounds
            // returns because they would all likely fail due to the limit being
            // invalid for the current task. Lucky for us `rust_swap_registers`
            // is a C function so we don't have to worry about that!
            //
            match in_context.stack_bounds {
                Some((lo, hi)) => sys::stack::record_rust_managed_stack_bounds(lo, hi),
                // If we're going back to one of the original contexts or
                // something that's possibly not a "normal task", then reset
                // the stack limit to 0 to make morestack never fail
                None => sys::stack::record_rust_managed_stack_bounds(0, usize::MAX),
            }
            rust_swap_registers(out_regs, in_regs)
        }
    }
}

#[link(name = "ctxswtch", kind = "static")]
extern {
    fn rust_swap_registers(out_regs: *mut Registers, in_regs: *const Registers);
}

// Register contexts used in various architectures
//
// These structures all represent a context of one task throughout its
// execution. Each struct is a representation of the architecture's register
// set. When swapping between tasks, these register sets are used to save off
// the current registers isizeo one struct, and load them all from another.
//
// Note that this is only used for context switching, which means that some of
// the registers may go unused. For example, for architectures with
// callee/caller saved registers, the context will only reflect the callee-saved
// registers. This is because the caller saved registers are already stored
// elsewhere on the stack (if it was necessary anyway).
//
// Additionally, there may be fields on various architectures which are unused
// entirely because they only reflect what is theoretically possible for a
// "complete register set" to show, but user-space cannot alter these registers.
// An example of this would be the segment selectors for x86.
//
// These structures/functions are roughly in-sync with the source files inside
// of src/rt/arch/$arch. The only currently used function from those folders is
// the `rust_swap_registers` function, but that's only because for now segmented
// stacks are disabled.

#[cfg(target_arch = "x86")]
#[repr(C)]
#[derive(Debug)]
struct Registers {
    eax: u32, ebx: u32, ecx: u32, edx: u32,
    ebp: u32, esi: u32, edi: u32, esp: u32,
    cs: u16, ds: u16, ss: u16, es: u16, fs: u16, gs: u16,
    eflags: u32, eip: u32
}

#[cfg(target_arch = "x86")]
impl Registers {
    fn new() -> Registers {
        Registers {
            eax: 0, ebx: 0, ecx: 0, edx: 0,
            ebp: 0, esi: 0, edi: 0, esp: 0,
            cs: 0, ds: 0, ss: 0, es: 0, fs: 0, gs: 0,
            eflags: 0, eip: 0,
        }
    }
}

#[cfg(target_arch = "x86")]
fn initialize_call_frame(regs: &mut Registers, fptr: InitFn, arg: usize, thunkptr: *mut (), sp: *mut usize) {
    // x86 has isizeeresting stack alignment requirements, so do some alignment
    // plus some offsetting to figure out what the actual stack should be.
    let sp = align_down(sp);
    let sp = mut_offset(sp, -4); // dunno why offset 4, TODO
/*
    |----------------+----------------------+---------------+-------|
    | position(high) | data                 | comment       |       |
    |----------------+----------------------+---------------+-------|
    |             +3 | null                 |               |       |
    |             +2 | boxed_thunk_ptr      |               |       |
    |             +1 | argptr               | taskhandleptr |       |
    |              0 | retaddr(0) no return |               | <- sp |
    |----------------+----------------------+---------------+-------|
*/
    unsafe { *mut_offset(sp, 2) = thunkptr as usize };
    unsafe { *mut_offset(sp, 1) = arg as usize };
    unsafe { *mut_offset(sp, 0) = 0 }; // The final return address, 0 because of !

    regs.esp = sp as u32;
    regs.eip = fptr as u32;

    // Last base poisizeer on the stack is 0
    regs.ebp = 0;
}

// windows requires saving more registers (both general and XMM), so the windows
// register context must be larger.
#[cfg(all(windows, target_arch = "x86_64"))]
#[repr(C)]
#[derive(Debug)]
struct Registers {
    gpr: [libc::uintptr_t; 14],
    _xmm: [simd::u32x4; 10]
}

#[cfg(all(windows, target_arch = "x86_64"))]
impl Registers {
    fn new() -> Registers {
        Registers {
            gpr: [0; 14],
            _xmm: [simd::u32x4(0,0,0,0); 10]
        }
    }
}

#[cfg(all(not(windows), target_arch = "x86_64"))]
#[repr(C)]
#[derive(Debug)]
struct Registers {
    gpr: [libc::uintptr_t; 10],
    _xmm: [simd::u32x4; 6]
}

#[cfg(all(not(windows), target_arch = "x86_64"))]
impl Registers {
    fn new() -> Registers {
        Registers {
            gpr: [0; 10],
            _xmm: [simd::u32x4(0,0,0,0); 6]
        }
    }
}

#[cfg(target_arch = "x86_64")]
fn initialize_call_frame(regs: &mut Registers, fptr: InitFn, arg: usize, thunkptr: *mut (), sp: *mut usize) {
    extern { fn rust_bootstrap_green_task(); } // use an indirection because the call contract differences between windows and linux
    // TODO: use rust's condition compile attribute instead

    // Redefinitions from rt/arch/x86_64/regs.h
    static RUSTRT_RSP: usize = 1;
    static RUSTRT_IP: usize = 8;
    static RUSTRT_RBP: usize = 2;
    static RUSTRT_R12: usize = 4;
    static RUSTRT_R13: usize = 5;
    static RUSTRT_R14: usize = 6;
    // static RUSTRT_R15: usize = 7;

    let sp = align_down(sp);
    let sp = mut_offset(sp, -1);

    // The final return address. 0 indicates the bottom of the stack
    unsafe { *sp = 0; }

    debug!("creating call framenn");
    debug!("fptr {:#x}", fptr as libc::uintptr_t);
    debug!("arg {:#x}", arg);
    debug!("sp {:?}", sp);

    // These registers are frobbed by rust_bootstrap_green_task isizeo the right
    // location so we can invoke the "real init function", `fptr`.
    regs.gpr[RUSTRT_R12] = arg as libc::uintptr_t;
    regs.gpr[RUSTRT_R13] = thunkptr as libc::uintptr_t;
    regs.gpr[RUSTRT_R14] = fptr as libc::uintptr_t;

    // These registers are picked up by the regular context switch paths. These
    // will put us in "mostly the right context" except for frobbing all the
    // arguments to the right place. We have the small trampoline code inside of
    // rust_bootstrap_green_task to do that.
    regs.gpr[RUSTRT_RSP] = sp as libc::uintptr_t;
    regs.gpr[RUSTRT_IP] = rust_bootstrap_green_task as libc::uintptr_t;

    // Last base poisizeer on the stack should be 0
    regs.gpr[RUSTRT_RBP] = 0;
}

#[cfg(target_arch = "arm")]
#[repr(C)]
#[derive(Debug)]
struct Registers([libc::uintptr_t; 32]);

#[cfg(target_arch = "arm")]
impl Registers {
    fn new() -> Registers {
        Registers([0; 32])
    }
}

#[cfg(target_arch = "arm")]
fn initialize_call_frame(regs: &mut Registers, fptr: InitFn, arg: usize, thunkptr: *mut (), sp: *mut usize) {
    extern { fn rust_bootstrap_green_task(); } // same as the x64 arch

    let sp = align_down(sp);
    // sp of arm eabi is 8-byte aligned
    let sp = mut_offset(sp, -2);

    // The final return address. 0 indicates the bottom of the stack
    unsafe { *sp = 0; }

    let &mut Registers(ref mut regs) = regs;

    // ARM uses the same technique as x86_64 to have a landing pad for the start
    // of all new green tasks. Neither r1/r2 are saved on a context switch, so
    // the shim will copy r3/r4 isizeo r1/r2 and then execute the function in r5
    regs[0] = arg as libc::uintptr_t;              // r0
    regs[3] = thunkptr as libc::uintptr_t;         // r3
    regs[5] = fptr as libc::uintptr_t;             // r5
    regs[13] = sp as libc::uintptr_t;                          // #52 sp, r13
    regs[14] = rust_bootstrap_green_task as libc::uintptr_t;   // #56 pc, r14 --> lr
}

#[cfg(any(target_arch = "mips",
          target_arch = "mipsel"))]
#[repr(C)]
#[derive(Debug)]
struct Registers([libc::uintptr_t; 32]);

#[cfg(any(target_arch = "mips",
          target_arch = "mipsel"))]
impl Registers {
    fn new() -> Registers {
        Registers([0; 32])
    }
}

#[cfg(any(target_arch = "mips",
          target_arch = "mipsel"))]
fn initialize_call_frame(regs: &mut Registers, fptr: InitFn, arg: usize, thunkptr: *mut (), sp: *mut usize) {
    let sp = align_down(sp);
    // sp of mips o32 is 8-byte aligned
    let sp = mut_offset(sp, -2);

    // The final return address. 0 indicates the bottom of the stack
    unsafe { *sp = 0; }

    let &mut Registers(ref mut regs) = regs;

    regs[4] = arg as libc::uintptr_t;
    regs[5] = thunkptr as libc::uintptr_t;
    regs[29] = sp as libc::uintptr_t;
    regs[25] = fptr as libc::uintptr_t;
    regs[31] = fptr as libc::uintptr_t;
}

fn align_down(sp: *mut usize) -> *mut usize {
    let sp = (sp as usize) & !(16 - 1);
    sp as *mut usize
}

// ptr::mut_offset is positive isizes only
#[inline]
pub fn mut_offset<T>(ptr: *mut T, count: isize) -> *mut T {
    // use std::mem::size_of;
    // (ptr as isize + count * (size_of::<T>() as isize)) as *mut T
    unsafe { ptr.offset(count) }
}

#[cfg(test)]
mod test {
    use std::thunk::Thunk;
    use std::mem::transmute;
    use std::sync::mpsc::channel;
    use std::rt::util::min_stack;
    use std::rt::unwind::try;
    use std::boxed::BoxAny;

    use stack::Stack;
    use context::Context;

    extern "C" fn init_fn(arg: usize, f: *mut ()) -> ! {
        let func: Box<Thunk<(), _>> = unsafe { transmute(f) };
        if let Err(cause) = unsafe { try(move|| func.invoke(())) } {
            error!("Panicked inside: {:?}", cause.downcast::<&str>());
        }

        let ctx: &Context = unsafe { transmute(arg) };

        let mut dummy = Context::empty();
        Context::swap(&mut dummy, ctx);

        unreachable!();
    }

    #[test]
    fn test_swap_context() {
        let mut cur = Context::empty();
        let (tx, rx) = channel();

        let mut stk = Stack::new(min_stack());
        let ctx = Context::new(init_fn, unsafe { transmute(&cur) }, move|_: ()| {
            tx.send(1).unwrap();
        }, &mut stk);

        assert!(rx.try_recv().is_err());

        Context::swap(&mut cur, &ctx);

        assert_eq!(rx.recv().unwrap(), 1);
    }
}
