//! Non-local trap boundary for module calls.
//!
//! The host API trap service is non-returning by contract. The runtime installs
//! a `setjmp`/`longjmp` boundary around every module entry-point call so a trap
//! performs a non-local exit to the host without unwinding through module or C
//! frames (GOT Artifacts §8.4). The boundary is thread-local: the host
//! serializes module execution on one thread per generation.
//!
//! The setjmp call and its second return live in a small assembly shim. Rust
//! cannot attach LLVM's `returns_twice` attribute to an external `setjmp`
//! declaration, so a Rust `setjmp` call can be miscompiled after `longjmp`
//! resumes it. With the shim, Rust sees one ordinary return: `0` when the body
//! completed and `1` when a trap jumped back to the shim's saved context.
//!
//! When no boundary is installed (for example an embedding host calls a module
//! export directly), the trap service falls back to the executable behavior and
//! exits with code 70.

use std::cell::Cell;
use std::ffi::c_void;
use std::mem::MaybeUninit;
use std::os::raw::c_int;

/// Storage for a platform `jmp_buf`.
///
/// A `jmp_buf` is at most 256 bytes on the supported targets; the extra
/// alignment keeps the opaque buffer safe on every host.
#[repr(C, align(16))]
struct Boundary {
    words: [u64; 32],
}

thread_local! {
    static CURRENT: Cell<*mut Boundary> = const { Cell::new(std::ptr::null_mut()) };
    static LAST_TRAP: Cell<Option<(i32, u32)>> = const { Cell::new(None) };
}

// Runs one boundary body and returns `0` after a normal return or `1` after a
// `longjmp` to the context saved in `environment`.
unsafe extern "C" {
    fn stc_trap_boundary_run(
        environment: *mut c_void,
        body: extern "C" fn(*mut c_void),
        context: *mut c_void,
    ) -> c_int;
}

#[cfg(all(target_arch = "x86_64", windows))]
core::arch::global_asm!(
    r#"
    .globl stc_trap_boundary_run
    stc_trap_boundary_run:
        pushq %rbx
        pushq %r12
        subq $40, %rsp
        movq %rdx, %rbx
        movq %r8, %r12
        xorl %edx, %edx
        callq __intrinsic_setjmpex
        testl %eax, %eax
        jne 1f
        movq %r12, %rcx
        callq *%rbx
        xorl %eax, %eax
        jmp 2f
    1:
        movl $1, %eax
    2:
        addq $40, %rsp
        popq %r12
        popq %rbx
        retq
    "#,
    options(att_syntax)
);

#[cfg(all(target_arch = "x86_64", not(windows)))]
core::arch::global_asm!(
    r#"
    .globl stc_trap_boundary_run
    stc_trap_boundary_run:
        pushq %rbx
        pushq %r12
        subq $8, %rsp
        movq %rsi, %rbx
        movq %rdx, %r12
        callq _setjmp
        testl %eax, %eax
        jne 1f
        movq %r12, %rdi
        callq *%rbx
        xorl %eax, %eax
        jmp 2f
    1:
        movl $1, %eax
    2:
        addq $8, %rsp
        popq %r12
        popq %rbx
        retq
    "#,
    options(att_syntax)
);

#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(
    r#"
    .globl stc_trap_boundary_run
    stc_trap_boundary_run:
        stp x29, x30, [sp, #-32]!
        mov x29, sp
        stp x19, x20, [sp, #16]
        mov x19, x1
        mov x20, x2
        bl _setjmp
        cbnz w0, 1f
        mov x0, x20
        blr x19
        mov w0, #0
        b 2f
    1:
        mov w0, #1
    2:
        ldp x19, x20, [sp, #16]
        ldp x29, x30, [sp], #32
        ret
    "#
);

#[cfg(not(any(
    all(target_arch = "x86_64", any(windows, target_os = "linux")),
    all(target_arch = "aarch64", target_os = "linux")
)))]
compile_error!(
    "the trap boundary shim is only implemented for x86_64 Windows/Linux and aarch64 Linux"
);

unsafe extern "C" {
    fn longjmp(environment: *mut c_void, value: c_int) -> !;
}

/// A trap caught by the host boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Trap {
    /// Trap code from Runtime-and-Execution §2.3.
    pub(crate) code: i32,
    /// Span ID supplied by generated code.
    pub(crate) span_id: u32,
}

struct BoundaryState<T, F> {
    body: Option<F>,
    result: MaybeUninit<T>,
}

extern "C" fn boundary_trampoline<T, F: FnOnce() -> T>(context: *mut c_void) {
    // SAFETY: the context is the `BoundaryState<T, F>` passed to the shim by
    // `with_boundary` for this exact monomorphization.
    let state = unsafe { &mut *(context as *mut BoundaryState<T, F>) };
    let Some(body) = state.body.take() else {
        return;
    };
    let result = body();
    state.result.write(result);
}

/// Runs `body` inside a trap boundary.
///
/// On a trap, control returns here with [`Trap`] and the partially executed
/// call is abandoned. The caller must treat the generation as failed.
pub(crate) fn with_boundary<T, F: FnOnce() -> T>(body: F) -> Result<T, Trap> {
    let previous = CURRENT.with(Cell::get);
    let mut boundary = Boundary { words: [0; 32] };
    let mut state = BoundaryState {
        body: Some(body),
        result: MaybeUninit::uninit(),
    };
    let pointer = std::ptr::addr_of_mut!(boundary);
    CURRENT.with(|current| current.set(pointer));
    LAST_TRAP.with(|last| last.set(None));
    // SAFETY: the environment buffer is aligned and large enough for the
    // platform `jmp_buf`; the state outlives the shim call. The shim performs
    // the platform setjmp and reports its second return as status `1`.
    let status = unsafe {
        stc_trap_boundary_run(
            std::ptr::addr_of_mut!(boundary.words).cast(),
            boundary_trampoline::<T, F>,
            std::ptr::addr_of_mut!(state).cast(),
        )
    };
    CURRENT.with(|current| current.set(previous));
    if status == 0 {
        // SAFETY: a zero status means the trampoline ran and wrote the result.
        Ok(unsafe { state.result.assume_init() })
    } else {
        let (code, span_id) = LAST_TRAP.with(Cell::get).unwrap_or((0, 0));
        Err(Trap { code, span_id })
    }
}

/// Transfers control to the active boundary.
///
/// Returns `false` when no boundary is installed, in which case the caller
/// keeps the non-returning exit-70 behavior.
pub(crate) fn escape(code: i32, span_id: u32) -> bool {
    let boundary = CURRENT.with(Cell::get);
    if boundary.is_null() {
        return false;
    }
    LAST_TRAP.with(|last| last.set(Some((code, span_id))));
    // SAFETY: the pointer names the boundary installed by `with_boundary`,
    // whose shim frame is still live, and `longjmp` never returns.
    unsafe {
        longjmp(std::ptr::addr_of_mut!((*boundary).words).cast(), 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trap_returns_to_the_boundary() {
        let result = with_boundary(|| {
            assert!(escape(3, 7));
            1
        });
        assert_eq!(
            result,
            Err(Trap {
                code: 3,
                span_id: 7
            })
        );
        assert_eq!(with_boundary(|| 9), Ok(9));
    }
}
