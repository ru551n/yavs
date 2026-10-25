//! IR generators for the LLVM risk spike. They mimic what `yavs-ir` lowering
//! will emit: checked i64 arithmetic whose overflow path calls back into the
//! Rust runtime, and process bodies that read and write signal state through
//! a pointer.
//!
//! Every generated function returns `i32`: 0 for normal completion, 1 when a
//! runtime error was reported through the callback. The callback returns
//! normally; JIT frames are never unwound through.

use std::ffi::CStr;

use llvm_sys::LLVMIntPredicate;
use llvm_sys::core::*;
use llvm_sys::prelude::*;

use crate::Module;

/// Name of the runtime callback the generated code calls on overflow:
/// `extern "C" fn(stmt: i64)`, where `stmt` identifies the failing statement.
pub const OVERFLOW_CALLBACK: &str = "yavs_rt_overflow";

/// Signal state as the `process_body` spike sees it.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SigState {
    pub cur: i64,
    pub next: i64,
    pub event: i32,
    pub runs: i32,
}

/// `checked_sum(data: *const i64, n: i64, out: *mut i64) -> i32`
pub type SumFn = unsafe extern "C" fn(*const i64, i64, *mut i64) -> i32;
/// `process_body(state: *mut SigState) -> i32`
pub type ProcessFn = unsafe extern "C" fn(*mut SigState) -> i32;
/// `chain(state: *mut [i64; CHAIN_FIELDS]) -> i32`
pub type ChainFn = unsafe extern "C" fn(*mut i64) -> i32;

/// Number of i64 fields in the state the `chain` function works on.
pub const CHAIN_FIELDS: usize = 64;

struct Gen {
    ctx: LLVMContextRef,
    module: LLVMModuleRef,
    b: LLVMBuilderRef,
    i32: LLVMTypeRef,
    i64: LLVMTypeRef,
    ptr: LLVMTypeRef,
    sadd: (LLVMTypeRef, LLVMValueRef),
    smul: (LLVMTypeRef, LLVMValueRef),
    overflow: (LLVMTypeRef, LLVMValueRef),
}

impl Gen {
    fn new(m: &Module) -> Gen {
        let (ctx, module) = (m.context(), m.raw());
        // SAFETY: ctx and module are live for the lifetime of `m`; the builder
        // is disposed in Drop.
        unsafe {
            let i64 = LLVMInt64TypeInContext(ctx);
            let intrinsic = |name: &CStr| {
                let id = LLVMLookupIntrinsicID(name.as_ptr(), name.count_bytes());
                assert_ne!(id, 0, "unknown intrinsic {name:?}");
                let mut tys = [i64];
                let ty = LLVMIntrinsicGetType(ctx, id, tys.as_mut_ptr(), 1);
                (
                    ty,
                    LLVMGetIntrinsicDeclaration(module, id, tys.as_mut_ptr(), 1),
                )
            };
            let mut params = [i64];
            let cb_ty = LLVMFunctionType(LLVMVoidTypeInContext(ctx), params.as_mut_ptr(), 1, 0);
            let name = std::ffi::CString::new(OVERFLOW_CALLBACK).unwrap();
            Gen {
                ctx,
                module,
                b: LLVMCreateBuilderInContext(ctx),
                i32: LLVMInt32TypeInContext(ctx),
                i64,
                ptr: LLVMPointerTypeInContext(ctx, 0),
                sadd: intrinsic(c"llvm.sadd.with.overflow"),
                smul: intrinsic(c"llvm.smul.with.overflow"),
                overflow: (cb_ty, LLVMAddFunction(module, name.as_ptr(), cb_ty)),
            }
        }
    }

    unsafe fn function(
        &self,
        name: &CStr,
        ret: LLVMTypeRef,
        params: &mut [LLVMTypeRef],
    ) -> LLVMValueRef {
        // SAFETY: plain function creation.
        unsafe {
            let ty = LLVMFunctionType(ret, params.as_mut_ptr(), params.len() as u32, 0);
            LLVMAddFunction(self.module, name.as_ptr(), ty)
        }
    }

    unsafe fn block(&self, f: LLVMValueRef) -> LLVMBasicBlockRef {
        // SAFETY: f is a function in our context.
        unsafe { LLVMAppendBasicBlockInContext(self.ctx, f, c"".as_ptr()) }
    }

    unsafe fn call(
        &self,
        (ty, f): (LLVMTypeRef, LLVMValueRef),
        args: &mut [LLVMValueRef],
    ) -> LLVMValueRef {
        // SAFETY: argument types match `ty` at every call site below.
        unsafe {
            LLVMBuildCall2(
                self.b,
                ty,
                f,
                args.as_mut_ptr(),
                args.len() as u32,
                c"".as_ptr(),
            )
        }
    }

    /// Emits a checked binary op. Branches to a fresh block that reports
    /// `stmt` and returns 1 on overflow; the builder ends up in the
    /// continuation block. Returns the result.
    unsafe fn checked(
        &self,
        f: LLVMValueRef,
        op: (LLVMTypeRef, LLVMValueRef),
        a: LLVMValueRef,
        b: LLVMValueRef,
        stmt: i64,
    ) -> LLVMValueRef {
        // SAFETY: well-formed IR built in the current block.
        unsafe {
            let r = self.call(op, &mut [a, b]);
            let v = LLVMBuildExtractValue(self.b, r, 0, c"".as_ptr());
            let o = LLVMBuildExtractValue(self.b, r, 1, c"".as_ptr());
            let trap = self.block(f);
            let cont = self.block(f);
            LLVMBuildCondBr(self.b, o, trap, cont);
            LLVMPositionBuilderAtEnd(self.b, trap);
            self.call(self.overflow, &mut [self.c64(stmt)]);
            LLVMBuildRet(self.b, self.c32(1));
            LLVMPositionBuilderAtEnd(self.b, cont);
            v
        }
    }

    fn c64(&self, v: i64) -> LLVMValueRef {
        // SAFETY: constant creation.
        unsafe { LLVMConstInt(self.i64, v as u64, 1) }
    }

    fn c32(&self, v: i32) -> LLVMValueRef {
        // SAFETY: constant creation.
        unsafe { LLVMConstInt(self.i32, v as u64, 1) }
    }
}

impl Drop for Gen {
    fn drop(&mut self) {
        // SAFETY: we own the builder.
        unsafe { LLVMDisposeBuilder(self.b) }
    }
}

/// Adds `checked_sum`: sums `n` i64s with an overflow check per element.
/// On overflow it calls the callback with the element index and returns 1.
pub fn add_checked_sum(m: &Module) {
    let g = Gen::new(m);
    // SAFETY: well-formed IR; the verifier runs in the tests.
    unsafe {
        let f = g.function(c"checked_sum", g.i32, &mut [g.ptr, g.i64, g.ptr]);
        let (data, n, out) = (LLVMGetParam(f, 0), LLVMGetParam(f, 1), LLVMGetParam(f, 2));
        let entry = g.block(f);
        let head = g.block(f);
        let body = g.block(f);
        let exit = g.block(f);

        LLVMPositionBuilderAtEnd(g.b, entry);
        LLVMBuildBr(g.b, head);

        LLVMPositionBuilderAtEnd(g.b, head);
        let i = LLVMBuildPhi(g.b, g.i64, c"i".as_ptr());
        let acc = LLVMBuildPhi(g.b, g.i64, c"acc".as_ptr());
        let done = LLVMBuildICmp(g.b, LLVMIntPredicate::LLVMIntSGE, i, n, c"".as_ptr());
        LLVMBuildCondBr(g.b, done, exit, body);

        LLVMPositionBuilderAtEnd(g.b, body);
        let p = LLVMBuildGEP2(g.b, g.i64, data, [i].as_mut_ptr(), 1, c"".as_ptr());
        let v = LLVMBuildLoad2(g.b, g.i64, p, c"".as_ptr());
        // Not `checked`: the trap reports the loop index, a value, not a constant.
        let r = g.call(g.sadd, &mut [acc, v]);
        let s = LLVMBuildExtractValue(g.b, r, 0, c"".as_ptr());
        let o = LLVMBuildExtractValue(g.b, r, 1, c"".as_ptr());
        let trap = g.block(f);
        let cont = g.block(f);
        LLVMBuildCondBr(g.b, o, trap, cont);
        LLVMPositionBuilderAtEnd(g.b, trap);
        g.call(g.overflow, &mut [i]);
        LLVMBuildRet(g.b, g.c32(1));

        LLVMPositionBuilderAtEnd(g.b, cont);
        let i1 = LLVMBuildAdd(g.b, i, g.c64(1), c"".as_ptr());
        LLVMBuildBr(g.b, head);

        LLVMAddIncoming(
            i,
            [g.c64(0), i1].as_mut_ptr(),
            [entry, cont].as_mut_ptr(),
            2,
        );
        LLVMAddIncoming(
            acc,
            [g.c64(0), s].as_mut_ptr(),
            [entry, cont].as_mut_ptr(),
            2,
        );

        LLVMPositionBuilderAtEnd(g.b, exit);
        LLVMBuildStore(g.b, acc, out);
        LLVMBuildRet(g.b, g.c32(0));
    }
}

/// Adds `low_half(p: *const i64) -> i64`: the low 32 bits of `*p`, written as a load and a mask
/// (the form the generator makes of the low half of a fat address).
pub fn add_low_half(m: &Module) {
    let g = Gen::new(m);
    // SAFETY: well-formed IR; the verifier runs in the tests.
    unsafe {
        let f = g.function(c"low_half", g.i64, &mut [g.ptr]);
        let entry = g.block(f);
        LLVMPositionBuilderAtEnd(g.b, entry);
        let v = LLVMBuildLoad2(g.b, g.i64, LLVMGetParam(f, 0), c"".as_ptr());
        let lo = LLVMBuildAnd(g.b, v, g.c64(0xffff_ffff), c"".as_ptr());
        LLVMBuildRet(g.b, lo);
    }
}

/// Adds `buffer(x: i64, sink: extern "C" fn(*const i64)) -> i32`: fills a stack buffer of `n`
/// words with `x + k` through byte offsets, hands its address to `sink` and returns 0. The shape
/// of the buffer a native frame's exits record the registers in: thousands of stores to
/// addresses made from one alloca that escapes to a call.
pub fn add_buffer(m: &Module, n: usize) {
    let g = Gen::new(m);
    // SAFETY: well-formed IR; the verifier runs in the tests.
    unsafe {
        let f = g.function(c"buffer", g.i32, &mut [g.i64, g.ptr]);
        LLVMPositionBuilderAtEnd(g.b, g.block(f));
        let buf = LLVMBuildAlloca(g.b, LLVMArrayType2(g.i64, n as u64), c"".as_ptr());
        let x = LLVMGetParam(f, 0);
        for k in 0..n {
            let at = LLVMBuildGEP2(
                g.b,
                LLVMInt8TypeInContext(g.ctx),
                buf,
                [g.c64(8 * k as i64)].as_mut_ptr(),
                1,
                c"".as_ptr(),
            );
            let v = LLVMBuildAdd(g.b, x, g.c64(k as i64), c"".as_ptr());
            LLVMSetAlignment(LLVMBuildStore(g.b, v, at), 1);
        }
        let mut params = [g.ptr];
        let ty = LLVMFunctionType(LLVMVoidTypeInContext(g.ctx), params.as_mut_ptr(), 1, 0);
        LLVMBuildCall2(
            g.b,
            ty,
            LLVMGetParam(f, 1),
            [buf].as_mut_ptr(),
            1,
            c"".as_ptr(),
        );
        LLVMBuildRet(g.b, g.c32(0));
    }
}

/// Adds `process_body`, a process touching signal state through a pointer:
/// `next = cur * 2 + 2` (checked, statements 1 and 2), `event = next != cur`,
/// `runs += 1`.
pub fn add_process_body(m: &Module) {
    let g = Gen::new(m);
    // SAFETY: well-formed IR; the verifier runs in the tests.
    unsafe {
        let mut fields = [g.i64, g.i64, g.i32, g.i32];
        let st = LLVMStructTypeInContext(g.ctx, fields.as_mut_ptr(), 4, 0);
        let f = g.function(c"process_body", g.i32, &mut [g.ptr]);
        let s = LLVMGetParam(f, 0);
        LLVMPositionBuilderAtEnd(g.b, g.block(f));
        let field = |i| LLVMBuildStructGEP2(g.b, st, s, i, c"".as_ptr());

        let cur = LLVMBuildLoad2(g.b, g.i64, field(0), c"cur".as_ptr());
        let twice = g.checked(f, g.smul, cur, g.c64(2), 1);
        let next = g.checked(f, g.sadd, twice, g.c64(2), 2);
        LLVMBuildStore(g.b, next, field(1));
        let ev = LLVMBuildICmp(g.b, LLVMIntPredicate::LLVMIntNE, next, cur, c"".as_ptr());
        LLVMBuildStore(g.b, LLVMBuildZExt(g.b, ev, g.i32, c"".as_ptr()), field(2));
        let runs = LLVMBuildLoad2(g.b, g.i32, field(3), c"".as_ptr());
        LLVMBuildStore(
            g.b,
            LLVMBuildAdd(g.b, runs, g.c32(1), c"".as_ptr()),
            field(3),
        );
        LLVMBuildRet(g.b, g.c32(0));
    }
}

/// Adds `chain`: `stmts` straight-line statements of the form
/// `state[w] := state[a] + (state[b] and 255)` (checked, overflow reports the
/// statement index), about 13 IR instructions and 2 basic blocks each. This is the
/// shape of a large process or subprogram body, used to measure JIT cost.
pub fn add_chain(m: &Module, stmts: usize) {
    let g = Gen::new(m);
    // SAFETY: well-formed IR; the verifier runs in the tests.
    unsafe {
        let f = g.function(c"chain", g.i32, &mut [g.ptr]);
        let s = LLVMGetParam(f, 0);
        LLVMPositionBuilderAtEnd(g.b, g.block(f));
        let slot = |i: usize| {
            LLVMBuildGEP2(
                g.b,
                g.i64,
                s,
                [g.c64(i as i64)].as_mut_ptr(),
                1,
                c"".as_ptr(),
            )
        };
        for k in 0..stmts {
            let (a, b, w) = chain_indices(k);
            let va = LLVMBuildLoad2(g.b, g.i64, slot(a), c"".as_ptr());
            let vb = LLVMBuildLoad2(g.b, g.i64, slot(b), c"".as_ptr());
            let vb = LLVMBuildAnd(g.b, vb, g.c64(0xff), c"".as_ptr());
            let r = g.checked(f, g.sadd, va, vb, k as i64);
            LLVMBuildStore(g.b, r, slot(w));
        }
        LLVMBuildRet(g.b, g.c32(0));
    }
}

/// Field indices (read a, read b, write) of `chain` statement `k`.
pub fn chain_indices(k: usize) -> (usize, usize, usize) {
    (
        (k * 3) % CHAIN_FIELDS,
        (k * 5 + 1) % CHAIN_FIELDS,
        (k * 7 + 2) % CHAIN_FIELDS,
    )
}

/// Reference semantics of `chain` (what the interpreter would compute):
/// returns `Err(stmt)` on the first overflow, leaving earlier writes in place.
pub fn chain_reference(state: &mut [i64; CHAIN_FIELDS], stmts: usize) -> Result<(), i64> {
    for k in 0..stmts {
        let (a, b, w) = chain_indices(k);
        state[w] = state[a].checked_add(state[b] & 0xff).ok_or(k as i64)?;
    }
    Ok(())
}
