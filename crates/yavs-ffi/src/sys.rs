//! The asm trampolines and the only unsafe code in the crate.
#![allow(unsafe_code)]

use std::ffi::c_void;
use std::mem::offset_of;

use crate::abi::{self, Frame};
use crate::{Arg, FfiError, MAX_ARGS, Ret, Signature};

// The asm below hardcodes these offsets.
const _: () = {
    assert!(offset_of!(Frame, int) == 0);
    assert!(offset_of!(Frame, float) == 64);
    assert!(offset_of!(Frame, stack) == 128);
    assert!(offset_of!(Frame, nstack) == 136);
    assert!(offset_of!(Frame, func) == 144);
    assert!(offset_of!(Frame, ret_int) == 152);
    assert!(offset_of!(Frame, ret_float) == 160);
    assert!(offset_of!(Frame, nfloat) == 168);
};

/// SysV x86_64. `rdi` = frame. rbx keeps the frame across the call,
/// rbp the entry stack; after three pushes `rsp` is 16-aligned again and
/// the stack area is rounded up to 16 bytes.
#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
#[unsafe(naked)]
unsafe extern "C" fn trampoline(frame: *mut Frame) {
    core::arch::naked_asm!(
        "push rbp",
        "mov rbp, rsp",
        "push rbx",
        "push r12",
        "mov rbx, rdi",
        "mov rcx, [rbx + 136]",
        "lea rax, [rcx * 8 + 15]",
        "and rax, -16",
        "sub rsp, rax",
        "mov rsi, [rbx + 128]",
        "mov rdi, rsp",
        "rep movsq",
        "movq xmm0, [rbx + 64]",
        "movq xmm1, [rbx + 72]",
        "movq xmm2, [rbx + 80]",
        "movq xmm3, [rbx + 88]",
        "movq xmm4, [rbx + 96]",
        "movq xmm5, [rbx + 104]",
        "movq xmm6, [rbx + 112]",
        "movq xmm7, [rbx + 120]",
        "mov rdi, [rbx]",
        "mov rsi, [rbx + 8]",
        "mov rdx, [rbx + 16]",
        "mov rcx, [rbx + 24]",
        "mov r8, [rbx + 32]",
        "mov r9, [rbx + 40]",
        "mov rax, [rbx + 168]",
        "call qword ptr [rbx + 144]",
        "mov [rbx + 152], rax",
        "movq [rbx + 160], xmm0",
        "lea rsp, [rbp - 16]",
        "pop r12",
        "pop rbx",
        "pop rbp",
        "ret",
    )
}

/// Win64. `rcx` = frame. rsi/rdi are callee-saved here and used by
/// `rep movsq`; four pushes plus `sub 8` realign `rsp`, then the stack
/// area (16-rounded) and 32 bytes of shadow space go below it.
#[cfg(all(target_arch = "x86_64", target_os = "windows"))]
#[unsafe(naked)]
unsafe extern "C" fn trampoline(frame: *mut Frame) {
    core::arch::naked_asm!(
        "push rbp",
        "mov rbp, rsp",
        "push rbx",
        "push rsi",
        "push rdi",
        "sub rsp, 8",
        "mov rbx, rcx",
        "mov rcx, [rbx + 136]",
        "lea rax, [rcx * 8 + 15]",
        "and rax, -16",
        "sub rsp, rax",
        "sub rsp, 32",
        "mov rsi, [rbx + 128]",
        "lea rdi, [rsp + 32]",
        "rep movsq",
        "movq xmm0, [rbx + 64]",
        "movq xmm1, [rbx + 72]",
        "movq xmm2, [rbx + 80]",
        "movq xmm3, [rbx + 88]",
        "mov rcx, [rbx]",
        "mov rdx, [rbx + 8]",
        "mov r8, [rbx + 16]",
        "mov r9, [rbx + 24]",
        "call qword ptr [rbx + 144]",
        "mov [rbx + 152], rax",
        "movq [rbx + 160], xmm0",
        "lea rsp, [rbp - 24]",
        "pop rdi",
        "pop rsi",
        "pop rbx",
        "pop rbp",
        "ret",
    )
}

/// AAPCS64. `x0` = frame, kept in x19 across the call; x29 holds the
/// entry `sp`. The stack area is an even number of 8-byte slots.
#[cfg(all(target_arch = "aarch64", target_os = "linux"))]
#[unsafe(naked)]
unsafe extern "C" fn trampoline(frame: *mut Frame) {
    core::arch::naked_asm!(
        "stp x29, x30, [sp, #-32]!",
        "mov x29, sp",
        "str x19, [sp, #16]",
        "mov x19, x0",
        "ldr x9, [x19, #136]",
        "add x10, x9, #1",
        "and x10, x10, #-2",
        "lsl x10, x10, #3",
        "sub sp, sp, x10",
        "ldr x11, [x19, #128]",
        "mov x12, sp",
        "cbz x9, 2f",
        "1:",
        "ldr x13, [x11], #8",
        "str x13, [x12], #8",
        "subs x9, x9, #1",
        "b.ne 1b",
        "2:",
        "ldp d0, d1, [x19, #64]",
        "ldp d2, d3, [x19, #80]",
        "ldp d4, d5, [x19, #96]",
        "ldp d6, d7, [x19, #112]",
        "ldp x0, x1, [x19]",
        "ldp x2, x3, [x19, #16]",
        "ldp x4, x5, [x19, #32]",
        "ldp x6, x7, [x19, #48]",
        "ldr x16, [x19, #144]",
        "blr x16",
        "str x0, [x19, #152]",
        "str d0, [x19, #160]",
        "mov sp, x29",
        "ldr x19, [sp, #16]",
        "ldp x29, x30, [sp], #32",
        "ret",
    )
}

/// A C function pointer bound to its signature. Constructing one is the
/// unsafe step; calling it is safe, so crates that forbid unsafe code
/// (the interpreter) can make foreign calls.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForeignFn {
    ptr: *const c_void,
    sig: Signature,
}

// SAFETY: the pointer is code, not data; `new`'s contract makes calling it
// from any thread the caller's promise, as for any C function.
unsafe impl Send for ForeignFn {}
unsafe impl Sync for ForeignFn {}

impl ForeignFn {
    /// Binds `ptr` to `sig`. Fails early on a null pointer or more than
    /// [`MAX_ARGS`] parameters.
    ///
    /// # Safety
    ///
    /// `ptr` must be a C function (`extern "C"` on Windows means the
    /// Win64 convention) whose type is exactly `sig`, and calling it with
    /// any argument values later passed to [`ForeignFn::call`], including
    /// pointer arguments, must be sound.
    pub unsafe fn new(ptr: *const c_void, sig: Signature) -> Result<Self, FfiError> {
        if ptr.is_null() {
            return Err(FfiError::NullFunction);
        }
        if sig.params.len() > MAX_ARGS {
            return Err(FfiError::TooManyArgs {
                count: sig.params.len(),
                max: MAX_ARGS,
            });
        }
        Ok(ForeignFn { ptr, sig })
    }

    /// Binds a Rust `extern "C"` function pointer. Safe: the signature
    /// follows from the pointer's type, and a safe `extern "C" fn` is
    /// sound to call with any argument values of its parameter types.
    pub fn from_fn<F: crate::CFnPtr>(f: F) -> ForeignFn {
        ForeignFn {
            ptr: f.addr(),
            sig: F::signature(),
        }
    }

    pub fn signature(&self) -> &Signature {
        &self.sig
    }

    /// Calls the function. `args` must match the signature in number and
    /// C type, or nothing is called.
    pub fn call(&self, args: &[Arg]) -> Result<Ret, FfiError> {
        if args.len() != self.sig.params.len() {
            return Err(FfiError::ArgCount {
                expected: self.sig.params.len(),
                found: args.len(),
            });
        }
        for (index, (arg, &expected)) in args.iter().zip(&self.sig.params).enumerate() {
            if arg.ctype() != expected {
                return Err(FfiError::ArgType {
                    index,
                    expected,
                    found: arg.ctype(),
                });
            }
        }
        let mut stack = Vec::new();
        let mut frame = Frame {
            int: [0; 8],
            float: [0; 8],
            stack: std::ptr::null(),
            nstack: 0,
            func: self.ptr,
            ret_int: 0,
            ret_float: 0,
            nfloat: 0,
        };
        abi::build(abi::HOST, args, &mut frame, &mut stack);
        frame.stack = stack.as_ptr();
        frame.nstack = stack.len();
        // SAFETY: the frame is fully initialised and `stack` outlives the
        // call; the trampoline loads it per the host ABI, calls `func`,
        // and restores every callee-saved register it touches. That the
        // callee itself is sound is `new`'s contract.
        unsafe { trampoline(&mut frame) };
        Ok(Ret::decode(self.sig.ret, frame.ret_int, frame.ret_float))
    }
}

/// One-shot form of [`ForeignFn::new`] plus [`ForeignFn::call`].
///
/// # Safety
///
/// As for [`ForeignFn::new`].
pub unsafe fn call(fn_ptr: *const c_void, sig: &Signature, args: &[Arg]) -> Result<Ret, FfiError> {
    // SAFETY: forwarded to the caller.
    unsafe { ForeignFn::new(fn_ptr, sig.clone()) }?.call(args)
}

/// C-like callees that write through pointers, for marshalling tests in crates that forbid
/// `unsafe` (yavs-interp). Not part of the API.
#[doc(hidden)]
pub mod testing {
    use std::ffi::c_void;

    /// `int64_t sum_double(int32_t *p, int64_t n)`: the sum, and doubles every element.
    pub extern "C" fn sum_double(p: *mut c_void, n: i64) -> i64 {
        // SAFETY: the caller passes `n` valid `int32_t`s.
        let s = unsafe { std::slice::from_raw_parts_mut(p.cast::<i32>(), n as usize) };
        let sum = s.iter().map(|&v| i64::from(v)).sum();
        s.iter_mut().for_each(|v| *v *= 2);
        sum
    }

    /// `void invert8(uint8_t *p)`: swaps '0' and '1' (codes 2 and 3) in 8 std_ulogic codes,
    /// everything else becomes 'X'.
    pub extern "C" fn invert8(p: *mut c_void) {
        // SAFETY: the caller passes 8 valid bytes.
        let s = unsafe { std::slice::from_raw_parts_mut(p.cast::<u8>(), 8) };
        s.iter_mut().for_each(|c| {
            *c = match *c {
                2 => 3,
                3 => 2,
                _ => 1,
            }
        });
    }

    #[repr(C)]
    pub struct Rec {
        pub a: i32,
        pub b: f64,
        pub c: u8,
    }

    /// `void rec_bump(struct { int32_t a; double b; uint8_t c; } *p)`.
    pub extern "C" fn rec_bump(p: *mut c_void) {
        // SAFETY: the caller passes a valid `Rec`.
        let r = unsafe { &mut *p.cast::<Rec>() };
        r.a += 1;
        r.b *= 2.0;
        r.c = 1;
    }

    /// `int32_t poke(int64_t *p)`: writes 99 through a non-null pointer; returns 1 for NULL.
    pub extern "C" fn poke(p: *mut c_void) -> i32 {
        if p.is_null() {
            return 1;
        }
        // SAFETY: the caller passes a valid `int64_t`.
        unsafe { *p.cast::<i64>() = 99 };
        0
    }

    /// `void bad_bool(uint8_t *p)`: writes 7, which is no boolean.
    pub extern "C" fn bad_bool(p: *mut c_void) {
        // SAFETY: the caller passes one valid byte.
        unsafe { *p.cast::<u8>() = 7 };
    }
}
