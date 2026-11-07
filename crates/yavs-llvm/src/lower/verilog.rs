//! The Verilog instructions (#776) of values of 1 to 64 bits computed in place, as the Cranelift
//! lowering does (`yavs-clift` `lower/verilog.rs`; `rt::verilog_inline` says which): bit for bit
//! with x and z where the operator allows, else on known operands with a slow path that steps the
//! instruction and goes on after it.

use llvm_sys::LLVMIntPredicate as IP;
use llvm_sys::core::*;
use llvm_sys::prelude::*;
use yavs_ir::{FOp, Inst, Reg, VBinOp, VCaseKind, VUnOp};

use super::{Cur, Gen};

type P = (LLVMValueRef, LLVMValueRef);

fn mask(w: u32) -> u64 {
    if w >= 64 { !0 } else { (1u64 << w) - 1 }
}

impl Gen {
    unsafe fn vld(&self, c: &Cur, r: Reg) -> P {
        // SAFETY: registers of the function.
        unsafe { (self.ld(c, r), self.ld(c, Reg(r.0 + 1))) }
    }

    unsafe fn vst(&self, c: &Cur, r: Reg, v: P) {
        // SAFETY: as `vld`.
        unsafe {
            self.st(c, r, v.0);
            self.st(c, Reg(r.0 + 1), v.1);
        }
    }

    unsafe fn vmask(&self, v: LLVMValueRef, w: u32) -> LLVMValueRef {
        // SAFETY: i64 values.
        unsafe {
            match w {
                64.. => v,
                _ => LLVMBuildAnd(self.b, v, self.c64(mask(w) as i64), c"".as_ptr()),
            }
        }
    }

    unsafe fn andn(&self, a: LLVMValueRef, b: LLVMValueRef) -> LLVMValueRef {
        // SAFETY: i64 values.
        unsafe {
            let n = LLVMBuildNot(self.b, b, c"".as_ptr());
            LLVMBuildAnd(self.b, a, n, c"".as_ptr())
        }
    }

    unsafe fn vsext(&self, v: LLVMValueRef, w: u32) -> LLVMValueRef {
        if w >= 64 {
            return v;
        }
        // SAFETY: i64 values, shifts below 64.
        unsafe {
            let k = self.c64(i64::from(64 - w));
            let s = LLVMBuildShl(self.b, v, k, c"".as_ptr());
            LLVMBuildAShr(self.b, s, k, c"".as_ptr())
        }
    }

    unsafe fn vnot(&self, x: P, w: u32) -> P {
        // SAFETY: i64 values.
        unsafe {
            let n = LLVMBuildNot(self.b, x.0, c"".as_ptr());
            let a = LLVMBuildOr(self.b, n, x.1, c"".as_ptr());
            (self.vmask(a, w), x.1)
        }
    }

    unsafe fn vxor(&self, x: P, y: P) -> P {
        // SAFETY: i64 values.
        unsafe {
            let unk = LLVMBuildOr(self.b, x.1, y.1, c"".as_ptr());
            let d = LLVMBuildXor(self.b, x.0, y.0, c"".as_ptr());
            (LLVMBuildOr(self.b, d, unk, c"".as_ptr()), unk)
        }
    }

    unsafe fn vbit(&self, b: LLVMValueRef) -> P {
        // SAFETY: an i1.
        unsafe { (self.zext(b), self.c64(0)) }
    }

    /// The slow path unless `bad`, made at the first guard.
    unsafe fn vguard(
        &self,
        c: &Cur,
        bad: LLVMValueRef,
        sp: &mut Option<(LLVMBasicBlockRef, LLVMBasicBlockRef)>,
        pc: u32,
        inst: &Inst,
    ) {
        // SAFETY: blocks of the function.
        unsafe {
            let slow = match *sp {
                Some((s, _)) => s,
                None => {
                    let p = self.slow_path(c, pc, inst);
                    *sp = Some(p);
                    p.0
                }
            };
            let ok = LLVMBuildNot(self.b, bad, c"".as_ptr());
            self.guard(c, ok, slow);
        }
    }

    unsafe fn vknown(
        &self,
        c: &Cur,
        bs: &[LLVMValueRef],
        sp: &mut Option<(LLVMBasicBlockRef, LLVMBasicBlockRef)>,
        pc: u32,
        inst: &Inst,
    ) {
        // SAFETY: i64 values.
        unsafe {
            let mut any = bs[0];
            for &b in &bs[1..] {
                any = LLVMBuildOr(self.b, any, b, c"".as_ptr());
            }
            let bad = self.icmp(IP::LLVMIntNE, any, self.c64(0));
            self.vguard(c, bad, sp, pc, inst);
        }
    }

    /// A Verilog instruction not computed in place: the interpreter's own function for it
    /// (`yavs_rt_verilog`), the generic step only when that fails.
    pub(super) unsafe fn verilog_call(&self, c: &Cur, pc: u32, inst: &Inst) {
        // SAFETY: `vstep` has the run-function signature.
        unsafe {
            if c.nat.is_some() {
                return self.step(c, pc, inst);
            }
            self.call_go(
                c,
                self.fn_ty,
                self.vstep,
                &mut [c.env, c.regs, c.mem, self.c32(pc)],
                false,
            )
        }
    }

    /// `inst` in place when `rt::verilog_inline` holds (and not on a native frame); `false`
    /// with nothing generated otherwise.
    pub(super) unsafe fn verilog(&self, c: &Cur, pc: u32, inst: &Inst) -> bool {
        if c.nat.is_some() || !crate::rt::verilog_inline(inst) {
            return false;
        }
        let mut sp = None;
        // SAFETY: registers, values and blocks of the function being built.
        unsafe {
            let b = self.b;
            match *inst {
                Inst::VUn { op, w, dst, a } => {
                    let x = self.vld(c, a);
                    let v = if op == VUnOp::Not {
                        self.vnot(x, w)
                    } else {
                        self.vknown(c, &[x.1], &mut sp, pc, inst);
                        let z = self.c64(0);
                        match op {
                            VUnOp::Neg => {
                                let n = LLVMBuildNeg(b, x.0, c"".as_ptr());
                                (self.vmask(n, w), z)
                            }
                            VUnOp::LNot | VUnOp::RedNor => {
                                self.vbit(self.icmp(IP::LLVMIntEQ, x.0, z))
                            }
                            VUnOp::RedOr => self.vbit(self.icmp(IP::LLVMIntNE, x.0, z)),
                            VUnOp::RedAnd => {
                                self.vbit(self.icmp(IP::LLVMIntEQ, x.0, self.c64(mask(w) as i64)))
                            }
                            VUnOp::RedNand => {
                                self.vbit(self.icmp(IP::LLVMIntNE, x.0, self.c64(mask(w) as i64)))
                            }
                            _ => {
                                // The parity of the bits, folded.
                                let mut p = x.0;
                                for k in [32, 16, 8, 4, 2, 1] {
                                    let s = LLVMBuildLShr(b, p, self.c64(k), c"".as_ptr());
                                    p = LLVMBuildXor(b, p, s, c"".as_ptr());
                                }
                                let p = LLVMBuildAnd(b, p, self.c64(1), c"".as_ptr());
                                let want = i64::from(op == VUnOp::RedXor);
                                self.vbit(self.icmp(IP::LLVMIntEQ, p, self.c64(want)))
                            }
                        }
                    };
                    self.vst(c, dst, v);
                }
                Inst::VBin {
                    op,
                    signed,
                    w,
                    dst,
                    a,
                    b: rb,
                    ..
                } => {
                    let (x, y) = (self.vld(c, a), self.vld(c, rb));
                    let v = match op {
                        VBinOp::And => {
                            let z1 = LLVMBuildOr(b, x.0, x.1, c"".as_ptr());
                            let z2 = LLVMBuildOr(b, y.0, y.1, c"".as_ptr());
                            let nz = LLVMBuildAnd(b, z1, z2, c"".as_ptr());
                            let u = LLVMBuildOr(b, x.1, y.1, c"".as_ptr());
                            (nz, LLVMBuildAnd(b, u, nz, c"".as_ptr()))
                        }
                        VBinOp::Or => {
                            let one = LLVMBuildOr(
                                b,
                                self.andn(x.0, x.1),
                                self.andn(y.0, y.1),
                                c"".as_ptr(),
                            );
                            let u = LLVMBuildOr(b, x.1, y.1, c"".as_ptr());
                            let unk = self.andn(u, one);
                            (LLVMBuildOr(b, one, unk, c"".as_ptr()), unk)
                        }
                        VBinOp::Xor => self.vxor(x, y),
                        VBinOp::Xnor => self.vnot(self.vxor(x, y), w),
                        VBinOp::CaseEq | VBinOp::CaseNe => {
                            let ea = self.icmp(IP::LLVMIntEQ, x.0, y.0);
                            let eb = self.icmp(IP::LLVMIntEQ, x.1, y.1);
                            let e = LLVMBuildAnd(b, ea, eb, c"".as_ptr());
                            let e = match op {
                                VBinOp::CaseNe => LLVMBuildNot(b, e, c"".as_ptr()),
                                _ => e,
                            };
                            self.vbit(e)
                        }
                        _ => {
                            self.vknown(c, &[x.1, y.1], &mut sp, pc, inst);
                            let z = self.c64(0);
                            // Shifts by 64 or more are poison in LLVM: the amount is clamped
                            // and the result selected.
                            let big =
                                |lim: u32| self.icmp(IP::LLVMIntUGE, y.0, self.c64(i64::from(lim)));
                            let k63 = LLVMBuildAnd(b, y.0, self.c64(63), c"".as_ptr());
                            let v = match op {
                                VBinOp::Add => LLVMBuildAdd(b, x.0, y.0, c"".as_ptr()),
                                VBinOp::Sub => LLVMBuildSub(b, x.0, y.0, c"".as_ptr()),
                                VBinOp::Mul => LLVMBuildMul(b, x.0, y.0, c"".as_ptr()),
                                VBinOp::Shl => {
                                    let s = LLVMBuildShl(b, x.0, k63, c"".as_ptr());
                                    LLVMBuildSelect(b, big(w), z, s, c"".as_ptr())
                                }
                                VBinOp::Shr => {
                                    let s = LLVMBuildLShr(b, x.0, k63, c"".as_ptr());
                                    LLVMBuildSelect(b, big(w), z, s, c"".as_ptr())
                                }
                                VBinOp::Ashr if signed => {
                                    let k = LLVMBuildSelect(
                                        b,
                                        big(64),
                                        self.c64(63),
                                        k63,
                                        c"".as_ptr(),
                                    );
                                    LLVMBuildAShr(b, self.vsext(x.0, w), k, c"".as_ptr())
                                }
                                VBinOp::Ashr => {
                                    let s = LLVMBuildLShr(b, x.0, k63, c"".as_ptr());
                                    LLVMBuildSelect(b, big(w), z, s, c"".as_ptr())
                                }
                                VBinOp::Lt | VBinOp::Le | VBinOp::Gt | VBinOp::Ge => {
                                    let p = match (op, signed) {
                                        (VBinOp::Lt, true) => IP::LLVMIntSLT,
                                        (VBinOp::Le, true) => IP::LLVMIntSLE,
                                        (VBinOp::Gt, true) => IP::LLVMIntSGT,
                                        (VBinOp::Ge, true) => IP::LLVMIntSGE,
                                        (VBinOp::Lt, false) => IP::LLVMIntULT,
                                        (VBinOp::Le, false) => IP::LLVMIntULE,
                                        (VBinOp::Gt, false) => IP::LLVMIntUGT,
                                        _ => IP::LLVMIntUGE,
                                    };
                                    let (p1, q1) = match signed {
                                        true => (self.vsext(x.0, w), self.vsext(y.0, w)),
                                        false => (x.0, y.0),
                                    };
                                    self.zext(self.icmp(p, p1, q1))
                                }
                                VBinOp::Eq => self.zext(self.icmp(IP::LLVMIntEQ, x.0, y.0)),
                                VBinOp::Ne => self.zext(self.icmp(IP::LLVMIntNE, x.0, y.0)),
                                VBinOp::LAnd | VBinOp::LOr => {
                                    let p = self.icmp(IP::LLVMIntNE, x.0, z);
                                    let q = self.icmp(IP::LLVMIntNE, y.0, z);
                                    let r = match op {
                                        VBinOp::LAnd => LLVMBuildAnd(b, p, q, c"".as_ptr()),
                                        _ => self.or(p, q),
                                    };
                                    self.zext(r)
                                }
                                _ => unreachable!("verilog_inline"),
                            };
                            match op.is_bit() {
                                true => (v, z),
                                false => (self.vmask(v, w), z),
                            }
                        }
                    };
                    self.vst(c, dst, v);
                }
                Inst::VExt {
                    signed,
                    w,
                    wa,
                    dst,
                    a,
                } => {
                    let x = self.vld(c, a);
                    let v = if w <= wa {
                        (self.vmask(x.0, w), self.vmask(x.1, w))
                    } else if signed {
                        (
                            self.vmask(self.vsext(x.0, wa), w),
                            self.vmask(self.vsext(x.1, wa), w),
                        )
                    } else {
                        x
                    };
                    self.vst(c, dst, v);
                }
                Inst::VCat {
                    wlo, dst, hi, lo, ..
                } => {
                    let (h, l) = (self.vld(c, hi), self.vld(c, lo));
                    let k = self.c64(i64::from(wlo));
                    let ha = LLVMBuildShl(b, h.0, k, c"".as_ptr());
                    let hb = LLVMBuildShl(b, h.1, k, c"".as_ptr());
                    let v = (
                        LLVMBuildOr(b, ha, l.0, c"".as_ptr()),
                        LLVMBuildOr(b, hb, l.1, c"".as_ptr()),
                    );
                    self.vst(c, dst, v);
                }
                Inst::VMerge { dst, a, b: rb, .. } => {
                    let (x, y) = (self.vld(c, a), self.vld(c, rb));
                    let d = LLVMBuildXor(b, x.0, y.0, c"".as_ptr());
                    let u = LLVMBuildOr(b, x.1, y.1, c"".as_ptr());
                    let diff = LLVMBuildOr(b, d, u, c"".as_ptr());
                    let keep = self.andn(x.0, diff);
                    self.vst(c, dst, (LLVMBuildOr(b, keep, diff, c"".as_ptr()), diff));
                }
                Inst::VSel { w, n, dst, src, at } => {
                    let x = self.vld(c, src);
                    let k = self.ld(c, at);
                    let bad = self.icmp(IP::LLVMIntUGT, k, self.c64(i64::from(w - n)));
                    self.vguard(c, bad, &mut sp, pc, inst);
                    let p = LLVMBuildLShr(b, x.0, k, c"".as_ptr());
                    let q = LLVMBuildLShr(b, x.1, k, c"".as_ptr());
                    self.vst(c, dst, (self.vmask(p, n), self.vmask(q, n)));
                }
                Inst::VIndex {
                    signed,
                    desc,
                    w,
                    adj,
                    dst,
                    idx,
                } => {
                    let x = self.vld(c, idx);
                    self.vknown(c, &[x.1], &mut sp, pc, inst);
                    let k = if signed { self.vsext(x.0, w) } else { x.0 };
                    let v = match desc {
                        true => LLVMBuildSub(b, k, self.c64(adj), c"".as_ptr()),
                        false => LLVMBuildSub(b, self.c64(adj), k, c"".as_ptr()),
                    };
                    self.st(c, dst, v);
                }
                Inst::VTruth { dst, a, .. } => {
                    let x = self.vld(c, a);
                    let one = self.icmp(IP::LLVMIntNE, self.andn(x.0, x.1), self.c64(0));
                    let known = self.icmp(IP::LLVMIntEQ, x.1, self.c64(0));
                    let t = LLVMBuildSelect(b, known, self.c64(0), self.c64(2), c"".as_ptr());
                    let t = LLVMBuildSelect(b, one, self.c64(1), t, c"".as_ptr());
                    self.st(c, dst, t);
                }
                Inst::VCase {
                    kind,
                    dst,
                    a,
                    b: rb,
                    ..
                } => {
                    let (x, y) = (self.vld(c, a), self.vld(c, rb));
                    let d1 = LLVMBuildXor(b, x.0, y.0, c"".as_ptr());
                    let d2 = LLVMBuildXor(b, x.1, y.1, c"".as_ptr());
                    let mut diff = LLVMBuildOr(b, d1, d2, c"".as_ptr());
                    if kind != VCaseKind::Case {
                        // z bits on either side match anything, and x bits for `casex`.
                        let mut care =
                            LLVMBuildOr(b, self.andn(x.1, x.0), self.andn(y.1, y.0), c"".as_ptr());
                        if kind == VCaseKind::Casex {
                            let x1 = LLVMBuildAnd(b, x.0, x.1, c"".as_ptr());
                            let x2 = LLVMBuildAnd(b, y.0, y.1, c"".as_ptr());
                            care = LLVMBuildOr(
                                b,
                                care,
                                LLVMBuildOr(b, x1, x2, c"".as_ptr()),
                                c"".as_ptr(),
                            );
                        }
                        diff = self.andn(diff, care);
                    }
                    let m = self.zext(self.icmp(IP::LLVMIntEQ, diff, self.c64(0)));
                    self.st(c, dst, m);
                }
                Inst::VCount { signed, w, dst, a } => {
                    let x = self.vld(c, a);
                    self.vknown(c, &[x.1], &mut sp, pc, inst);
                    let v = if signed {
                        let s = self.vsext(x.0, w);
                        let pos = self.icmp(IP::LLVMIntSGT, s, self.c64(0));
                        LLVMBuildSelect(b, pos, s, self.c64(0), c"".as_ptr())
                    } else {
                        x.0
                    };
                    self.st(c, dst, v);
                }
                Inst::VRBin { op, dst, a, b: rb } => {
                    let (x, y) = (self.ldf(c, a), self.ldf(c, rb));
                    let v = match op {
                        FOp::Add => LLVMBuildFAdd(b, x, y, c"".as_ptr()),
                        FOp::Sub => LLVMBuildFSub(b, x, y, c"".as_ptr()),
                        FOp::Mul => LLVMBuildFMul(b, x, y, c"".as_ptr()),
                        _ => LLVMBuildFDiv(b, x, y, c"".as_ptr()),
                    };
                    self.st(c, dst, LLVMBuildBitCast(b, v, self.i64, c"".as_ptr()));
                }
                Inst::VBack { cnt } => {
                    let n = LLVMBuildAdd(b, self.ld(c, cnt), self.c64(1), c"".as_ptr());
                    // At the limit the interpreter counts the jump and reports the loop.
                    let lim = self.c64(yavs_interp::VERILOG_LOOP_LIMIT as i64);
                    let bad = self.icmp(IP::LLVMIntUGT, n, lim);
                    self.vguard(c, bad, &mut sp, pc, inst);
                    self.st(c, cnt, n);
                }
                _ => unreachable!("verilog_inline"),
            }
            if let Some((_, join)) = sp {
                self.join(c, join);
            }
        }
        true
    }
}
