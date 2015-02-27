
/// This module is copied from `libstd/sys/common/stack.rs`.

#[allow(dead_code)]

pub mod stack {
    pub const RED_ZONE: usize = 20 * 1024;

    #[inline(always)]
    pub unsafe fn record_rust_managed_stack_bounds(stack_lo: usize, stack_hi: usize) {
        // When the old runtime had segmented stacks, it used a calculation that was
        // "limit + RED_ZONE + FUDGE". The red zone was for things like dynamic
        // symbol resolution, llvm function calls, etc. In theory this red zone
        // value is 0, but it matters far less when we have gigantic stacks because
        // we don't need to be so exact about our stack budget. The "fudge factor"
        // was because LLVM doesn't emit a stack check for functions < 256 bytes in
        // size. Again though, we have giant stacks, so we round all these
        // calculations up to the nice round number of 20k.
        record_sp_limit(stack_lo + RED_ZONE);

        return target_record_stack_bounds(stack_lo, stack_hi);

        #[cfg(not(windows))] #[inline(always)]
        unsafe fn target_record_stack_bounds(_stack_lo: usize, _stack_hi: usize) {}

        #[cfg(all(windows, target_arch = "x86"))] #[inline(always)]
        unsafe fn target_record_stack_bounds(stack_lo: usize, stack_hi: usize) {
            // stack range is at TIB: %fs:0x04 (top) and %fs:0x08 (bottom)
            asm!("mov $0, %fs:0x04" :: "r"(stack_hi) :: "volatile");
            asm!("mov $0, %fs:0x08" :: "r"(stack_lo) :: "volatile");
        }
        #[cfg(all(windows, target_arch = "x86_64"))] #[inline(always)]
        unsafe fn target_record_stack_bounds(stack_lo: usize, stack_hi: usize) {
            // stack range is at TIB: %gs:0x08 (top) and %gs:0x10 (bottom)
            asm!("mov $0, %gs:0x08" :: "r"(stack_hi) :: "volatile");
            asm!("mov $0, %gs:0x10" :: "r"(stack_lo) :: "volatile");
        }
    }

    /// Records the current limit of the stack as specified by `end`.
    ///
    /// This is stored in an OS-dependent location, likely inside of the thread
    /// local storage. The location that the limit is stored is a pre-ordained
    /// location because it's where LLVM has emitted code to check.
    ///
    /// Note that this cannot be called under normal circumstances. This function is
    /// changing the stack limit, so upon returning any further function calls will
    /// possibly be triggering the morestack logic if you're not careful.
    ///
    /// Also note that this and all of the inside functions are all flagged as
    /// "inline(always)" because they're messing around with the stack limits.  This
    /// would be unfortunate for the functions themselves to trigger a morestack
    /// invocation (if they were an actual function call).
    #[inline(always)]
    pub unsafe fn record_sp_limit(limit: usize) {
        return target_record_sp_limit(limit);

        // x86-64
        #[cfg(all(target_arch = "x86_64",
                  any(target_os = "macos", target_os = "ios")))]
        #[inline(always)]
        unsafe fn target_record_sp_limit(limit: usize) {
            asm!("movq $$0x60+90*8, %rsi
                  movq $0, %gs:(%rsi)" :: "r"(limit) : "rsi" : "volatile")
        }
        #[cfg(all(target_arch = "x86_64", target_os = "linux"))] #[inline(always)]
        unsafe fn target_record_sp_limit(limit: usize) {
            asm!("movq $0, %fs:112" :: "r"(limit) :: "volatile")
        }
        #[cfg(all(target_arch = "x86_64", target_os = "windows"))] #[inline(always)]
        unsafe fn target_record_sp_limit(_: usize) {
        }
        #[cfg(all(target_arch = "x86_64", target_os = "freebsd"))] #[inline(always)]
        unsafe fn target_record_sp_limit(limit: usize) {
            asm!("movq $0, %fs:24" :: "r"(limit) :: "volatile")
        }
        #[cfg(all(target_arch = "x86_64", target_os = "dragonfly"))]
        #[inline(always)]
        unsafe fn target_record_sp_limit(limit: usize) {
            asm!("movq $0, %fs:32" :: "r"(limit) :: "volatile")
        }

        // x86
        #[cfg(all(target_arch = "x86",
                  any(target_os = "macos", target_os = "ios")))]
        #[inline(always)]
        unsafe fn target_record_sp_limit(limit: usize) {
            asm!("movl $$0x48+90*4, %eax
                  movl $0, %gs:(%eax)" :: "r"(limit) : "eax" : "volatile")
        }
        #[cfg(all(target_arch = "x86",
                  any(target_os = "linux", target_os = "freebsd")))]
        #[inline(always)]
        unsafe fn target_record_sp_limit(limit: usize) {
            asm!("movl $0, %gs:48" :: "r"(limit) :: "volatile")
        }
        #[cfg(all(target_arch = "x86", target_os = "windows"))] #[inline(always)]
        unsafe fn target_record_sp_limit(_: usize) {
        }

        // mips, arm - Some brave soul can port these to inline asm, but it's over
        //             my head personally
        #[cfg(any(target_arch = "mips",
                  target_arch = "mipsel",
                  all(target_arch = "arm", not(target_os = "ios"))))]
        #[inline(always)]
        unsafe fn target_record_sp_limit(limit: usize) {
            use libc::c_void;
            return record_sp_limit(limit as *const c_void);
            extern {
                fn record_sp_limit(limit: *const c_void);
            }
        }

        // aarch64 - FIXME(AARCH64): missing...
        // powerpc - FIXME(POWERPC): missing...
        // arm-ios - iOS segmented stack is disabled for now, see related notes
        // openbsd - segmented stack is disabled
        #[cfg(any(target_arch = "aarch64",
                  target_arch = "powerpc",
                  all(target_arch = "arm", target_os = "ios"),
                  target_os = "bitrig",
                  target_os = "openbsd"))]
        unsafe fn target_record_sp_limit(_: usize) {
        }
    }

    /// The counterpart of the function above, this function will fetch the current
    /// stack limit stored in TLS.
    ///
    /// Note that all of these functions are meant to be exact counterparts of their
    /// brethren above, except that the operands are reversed.
    ///
    /// As with the setter, this function does not have a __morestack header and can
    /// therefore be called in a "we're out of stack" situation.
    #[inline(always)]
    pub unsafe fn get_sp_limit() -> usize {
        return target_get_sp_limit();

        // x86-64
        #[cfg(all(target_arch = "x86_64",
                  any(target_os = "macos", target_os = "ios")))]
        #[inline(always)]
        unsafe fn target_get_sp_limit() -> usize {
            let limit;
            asm!("movq $$0x60+90*8, %rsi
                  movq %gs:(%rsi), $0" : "=r"(limit) :: "rsi" : "volatile");
            return limit;
        }
        #[cfg(all(target_arch = "x86_64", target_os = "linux"))] #[inline(always)]
        unsafe fn target_get_sp_limit() -> usize {
            let limit;
            asm!("movq %fs:112, $0" : "=r"(limit) ::: "volatile");
            return limit;
        }
        #[cfg(all(target_arch = "x86_64", target_os = "windows"))] #[inline(always)]
        unsafe fn target_get_sp_limit() -> usize {
            return 1024;
        }
        #[cfg(all(target_arch = "x86_64", target_os = "freebsd"))] #[inline(always)]
        unsafe fn target_get_sp_limit() -> usize {
            let limit;
            asm!("movq %fs:24, $0" : "=r"(limit) ::: "volatile");
            return limit;
        }
        #[cfg(all(target_arch = "x86_64", target_os = "dragonfly"))]
        #[inline(always)]
        unsafe fn target_get_sp_limit() -> usize {
            let limit;
            asm!("movq %fs:32, $0" : "=r"(limit) ::: "volatile");
            return limit;
        }

        // x86
        #[cfg(all(target_arch = "x86",
                  any(target_os = "macos", target_os = "ios")))]
        #[inline(always)]
        unsafe fn target_get_sp_limit() -> usize {
            let limit;
            asm!("movl $$0x48+90*4, %eax
                  movl %gs:(%eax), $0" : "=r"(limit) :: "eax" : "volatile");
            return limit;
        }
        #[cfg(all(target_arch = "x86",
                  any(target_os = "linux", target_os = "freebsd")))]
        #[inline(always)]
        unsafe fn target_get_sp_limit() -> usize {
            let limit;
            asm!("movl %gs:48, $0" : "=r"(limit) ::: "volatile");
            return limit;
        }
        #[cfg(all(target_arch = "x86", target_os = "windows"))] #[inline(always)]
        unsafe fn target_get_sp_limit() -> usize {
            return 1024;
        }

        // mips, arm - Some brave soul can port these to inline asm, but it's over
        //             my head personally
        #[cfg(any(target_arch = "mips",
                  target_arch = "mipsel",
                  all(target_arch = "arm", not(target_os = "ios"))))]
        #[inline(always)]
        unsafe fn target_get_sp_limit() -> usize {
            use libc::c_void;
            return get_sp_limit() as usize;
            extern {
                fn get_sp_limit() -> *const c_void;
            }
        }

        // aarch64 - FIXME(AARCH64): missing...
        // powerpc - FIXME(POWERPC): missing...
        // arm-ios - iOS doesn't support segmented stacks yet.
        // openbsd - OpenBSD doesn't support segmented stacks.
        //
        // This function might be called by runtime though
        // so it is unsafe to unreachable, let's return a fixed constant.
        #[cfg(any(target_arch = "aarch64",
                  target_arch = "powerpc",
                  all(target_arch = "arm", target_os = "ios"),
                  target_os = "bitrig",
                  target_os = "openbsd"))]
        #[inline(always)]
        unsafe fn target_get_sp_limit() -> usize {
            1024
        }
    }
}
