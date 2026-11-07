//! The Verilog instructions (`Inst::VUn` to `Inst::VBack`, #776) of values of 1 to 64 bits,
//! two registers each (`yavs_ir::vregs`), computed in place: the bitwise operators, the `?:`
//! merge, extension, concatenation, `===` and the `case` matches bit for bit with x and z; the
//! arithmetic, relations, shifts, logical operators, reductions and index conversions on
//! operands with no x or z bit (both `b` planes 0), anything else on a slow path that steps the
//! instruction (the interpreter's word form, `yavs_logic::v4::w64`) and goes on after it. `rt::verilog_inline` is the same
//! test; the rest (nets, waits, tasks, wider values, `/`, `%`, `**`, `$clog2`, time and real
//! conversions) calls the interpreter's own function for Verilog instructions (`yavs_rt_verilog`),
//! not the generic step.

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{Block, InstBuilder, Value};
use yavs_ir::{FOp, Inst, Reg, VBinOp, VCaseKind, VUnOp};

use super::Gen;

type P = (Value, Value);

fn mask(w: u32) -> u64 {
    if w >= 64 { !0 } else { (1u64 << w) - 1 }
}

impl Gen<'_> {
    fn vld(&mut self, r: Reg) -> P {
        (self.ld(r), self.ld(Reg(r.0 + 1)))
    }

    fn vst(&mut self, r: Reg, v: P) {
        self.st(r, v.0);
        self.st(Reg(r.0 + 1), v.1);
    }

    fn vmask(&mut self, v: Value, w: u32) -> Value {
        match w {
            64.. => v,
            _ => self.b.ins().band_imm_s(v, mask(w) as i64),
        }
    }

    /// Goes on unless `bad`, else the slow path (the interpreter steps the instruction and the
    /// code goes on after it), made at the first guard.
    fn vguard(&mut self, bad: Value, sp: &mut Option<Block>, pc: u32, inst: &Inst) {
        let slow = match *sp {
            Some(s) => s,
            None => {
                let (slow, join) = self.slow_path(pc, inst);
                self.vjoin = Some(join);
                *sp = Some(slow);
                slow
            }
        };
        let ok = self.b.ins().bxor_imm_s(bad, 1);
        self.guard(ok, slow);
    }

    /// The slow path unless every one of `bs` (`b` planes) is 0.
    fn vknown(&mut self, bs: &[Value], sp: &mut Option<Block>, pc: u32, inst: &Inst) {
        let mut any = bs[0];
        for &b in &bs[1..] {
            any = self.b.ins().bor(any, b);
        }
        let bad = self.b.ins().icmp_imm_s(IntCC::NotEqual, any, 0);
        self.vguard(bad, sp, pc, inst);
    }

    /// A known bit from a truth value.
    fn vbit(&mut self, c: Value) -> P {
        let a = self.b.ins().uextend(I64, c);
        (a, self.c64(0))
    }

    /// The value as signed, from `w` bits.
    fn vsext(&mut self, v: Value, w: u32) -> Value {
        if w >= 64 {
            return v;
        }
        let s = self.b.ins().ishl_imm_s(v, i64::from(64 - w));
        self.b.ins().sshr_imm_s(s, i64::from(64 - w))
    }

    fn vnot(&mut self, x: P, w: u32) -> P {
        let n = self.b.ins().bnot(x.0);
        let a = self.b.ins().bor(n, x.1);
        (self.vmask(a, w), x.1)
    }

    fn vxor(&mut self, x: P, y: P) -> P {
        let unk = self.b.ins().bor(x.1, y.1);
        let d = self.b.ins().bxor(x.0, y.0);
        (self.b.ins().bor(d, unk), unk)
    }

    /// The truth of `x` (5.1.9): 1, 0, or 2 for unknown.
    fn vtruth(&mut self, x: P) -> Value {
        let one = self.b.ins().band_not(x.0, x.1);
        let is1 = self.b.ins().icmp_imm_s(IntCC::NotEqual, one, 0);
        let k = self.b.ins().icmp_imm_s(IntCC::Equal, x.1, 0);
        let (c0, c1, c2) = (self.c64(0), self.c64(1), self.c64(2));
        let t = self.b.ins().select(k, c0, c2);
        self.b.ins().select(is1, c1, t)
    }

    /// `inst` in place when it is a Verilog instruction this lowering computes; `false` (and
    /// nothing generated) when it is to be stepped.
    pub(super) fn verilog(&mut self, pc: u32, inst: &Inst) -> bool {
        if self.nat.is_some() || !yavs_llvm::rt::verilog_inline(inst) {
            return false;
        }
        let mut sp = None;
        match *inst {
            Inst::VUn { op, w, dst, a } => {
                let x = self.vld(a);
                let v = match op {
                    VUnOp::Not => self.vnot(x, w),
                    _ => {
                        self.vknown(&[x.1], &mut sp, pc, inst);
                        let c = match op {
                            VUnOp::Neg => {
                                let n = self.b.ins().ineg(x.0);
                                let n = self.vmask(n, w);
                                let z = self.c64(0);
                                self.vst(dst, (n, z));
                                self.vend(sp);
                                return true;
                            }
                            VUnOp::LNot => self.b.ins().icmp_imm_s(IntCC::Equal, x.0, 0),
                            VUnOp::RedAnd => {
                                self.b.ins().icmp_imm_s(IntCC::Equal, x.0, mask(w) as i64)
                            }
                            VUnOp::RedNand => {
                                self.b
                                    .ins()
                                    .icmp_imm_s(IntCC::NotEqual, x.0, mask(w) as i64)
                            }
                            VUnOp::RedOr => self.b.ins().icmp_imm_s(IntCC::NotEqual, x.0, 0),
                            VUnOp::RedNor => self.b.ins().icmp_imm_s(IntCC::Equal, x.0, 0),
                            VUnOp::RedXor | VUnOp::RedXnor => {
                                let p = self.b.ins().popcnt(x.0);
                                let p = self.b.ins().band_imm_s(p, 1);
                                let want = i64::from(op == VUnOp::RedXor);
                                self.b.ins().icmp_imm_s(IntCC::Equal, p, want)
                            }
                            _ => unreachable!("verilog_inline"),
                        };
                        self.vbit(c)
                    }
                };
                self.vst(dst, v);
            }
            Inst::VBin {
                op,
                signed,
                w,
                dst,
                a,
                b,
                ..
            } => {
                let (x, y) = (self.vld(a), self.vld(b));
                let v = match op {
                    VBinOp::And => {
                        let z1 = self.b.ins().bor(x.0, x.1);
                        let z2 = self.b.ins().bor(y.0, y.1);
                        // Not a known 0 on either side.
                        let nz = self.b.ins().band(z1, z2);
                        let u = self.b.ins().bor(x.1, y.1);
                        (nz, self.b.ins().band(u, nz))
                    }
                    VBinOp::Or => {
                        let o1 = self.b.ins().band_not(x.0, x.1);
                        let o2 = self.b.ins().band_not(y.0, y.1);
                        let one = self.b.ins().bor(o1, o2);
                        let u = self.b.ins().bor(x.1, y.1);
                        let unk = self.b.ins().band_not(u, one);
                        (self.b.ins().bor(one, unk), unk)
                    }
                    VBinOp::Xor => self.vxor(x, y),
                    VBinOp::Xnor => {
                        let r = self.vxor(x, y);
                        self.vnot(r, w)
                    }
                    VBinOp::CaseEq | VBinOp::CaseNe => {
                        let ea = self.b.ins().icmp(IntCC::Equal, x.0, y.0);
                        let eb = self.b.ins().icmp(IntCC::Equal, x.1, y.1);
                        let e = self.b.ins().band(ea, eb);
                        let e = match op {
                            VBinOp::CaseNe => self.b.ins().bxor_imm_s(e, 1),
                            _ => e,
                        };
                        self.vbit(e)
                    }
                    _ => {
                        self.vknown(&[x.1, y.1], &mut sp, pc, inst);
                        let z = self.c64(0);
                        let v = match op {
                            VBinOp::Add => self.b.ins().iadd(x.0, y.0),
                            VBinOp::Sub => self.b.ins().isub(x.0, y.0),
                            VBinOp::Mul => self.b.ins().imul(x.0, y.0),
                            VBinOp::Shl | VBinOp::Shr => {
                                let s = match op {
                                    VBinOp::Shl => self.b.ins().ishl(x.0, y.0),
                                    _ => self.b.ins().ushr(x.0, y.0),
                                };
                                // Cranelift shifts by the amount modulo 64.
                                let big = self.b.ins().icmp_imm_s(
                                    IntCC::UnsignedGreaterThanOrEqual,
                                    y.0,
                                    i64::from(w),
                                );
                                self.b.ins().select(big, z, s)
                            }
                            VBinOp::Ashr if signed => {
                                let sx = self.vsext(x.0, w);
                                let big =
                                    self.b.ins().icmp_imm_s(IntCC::UnsignedGreaterThan, y.0, 63);
                                let k63 = self.c64(63);
                                let k = self.b.ins().select(big, k63, y.0);
                                self.b.ins().sshr(sx, k)
                            }
                            VBinOp::Ashr => {
                                let s = self.b.ins().ushr(x.0, y.0);
                                let big = self.b.ins().icmp_imm_s(
                                    IntCC::UnsignedGreaterThanOrEqual,
                                    y.0,
                                    i64::from(w),
                                );
                                self.b.ins().select(big, z, s)
                            }
                            VBinOp::Lt | VBinOp::Le | VBinOp::Gt | VBinOp::Ge => {
                                let cc = match (op, signed) {
                                    (VBinOp::Lt, true) => IntCC::SignedLessThan,
                                    (VBinOp::Le, true) => IntCC::SignedLessThanOrEqual,
                                    (VBinOp::Gt, true) => IntCC::SignedGreaterThan,
                                    (VBinOp::Ge, true) => IntCC::SignedGreaterThanOrEqual,
                                    (VBinOp::Lt, false) => IntCC::UnsignedLessThan,
                                    (VBinOp::Le, false) => IntCC::UnsignedLessThanOrEqual,
                                    (VBinOp::Gt, false) => IntCC::UnsignedGreaterThan,
                                    _ => IntCC::UnsignedGreaterThanOrEqual,
                                };
                                let (p, q) = match signed {
                                    true => (self.vsext(x.0, w), self.vsext(y.0, w)),
                                    false => (x.0, y.0),
                                };
                                let c = self.b.ins().icmp(cc, p, q);
                                self.b.ins().uextend(I64, c)
                            }
                            VBinOp::Eq | VBinOp::Ne => {
                                let cc = match op {
                                    VBinOp::Eq => IntCC::Equal,
                                    _ => IntCC::NotEqual,
                                };
                                let c = self.b.ins().icmp(cc, x.0, y.0);
                                self.b.ins().uextend(I64, c)
                            }
                            VBinOp::LAnd | VBinOp::LOr => {
                                let p = self.b.ins().icmp_imm_s(IntCC::NotEqual, x.0, 0);
                                let q = self.b.ins().icmp_imm_s(IntCC::NotEqual, y.0, 0);
                                let c = match op {
                                    VBinOp::LAnd => self.b.ins().band(p, q),
                                    _ => self.b.ins().bor(p, q),
                                };
                                self.b.ins().uextend(I64, c)
                            }
                            _ => unreachable!("verilog_inline"),
                        };
                        let v = match op.is_bit() {
                            true => v,
                            false => self.vmask(v, w),
                        };
                        (v, z)
                    }
                };
                self.vst(dst, v);
            }
            Inst::VExt {
                signed,
                w,
                wa,
                dst,
                a,
            } => {
                let x = self.vld(a);
                let v = if w <= wa {
                    (self.vmask(x.0, w), self.vmask(x.1, w))
                } else if signed {
                    let (p, q) = (self.vsext(x.0, wa), self.vsext(x.1, wa));
                    (self.vmask(p, w), self.vmask(q, w))
                } else {
                    x
                };
                self.vst(dst, v);
            }
            Inst::VCat {
                wlo, dst, hi, lo, ..
            } => {
                let (h, l) = (self.vld(hi), self.vld(lo));
                let ha = self.b.ins().ishl_imm_s(h.0, i64::from(wlo));
                let hb = self.b.ins().ishl_imm_s(h.1, i64::from(wlo));
                let v = (self.b.ins().bor(ha, l.0), self.b.ins().bor(hb, l.1));
                self.vst(dst, v);
            }
            Inst::VMerge { dst, a, b, .. } => {
                let (x, y) = (self.vld(a), self.vld(b));
                let d = self.b.ins().bxor(x.0, y.0);
                let u = self.b.ins().bor(x.1, y.1);
                let diff = self.b.ins().bor(d, u);
                let keep = self.b.ins().band_not(x.0, diff);
                let v = (self.b.ins().bor(keep, diff), diff);
                self.vst(dst, v);
            }
            Inst::VSel { w, n, dst, src, at } => {
                let x = self.vld(src);
                let k = self.ld(at);
                // Inside the value (unsigned: a negative start fails too), else the interpreter.
                let bad = self
                    .b
                    .ins()
                    .icmp_imm_s(IntCC::UnsignedGreaterThan, k, i64::from(w - n));
                self.vguard(bad, &mut sp, pc, inst);
                let p = self.b.ins().ushr(x.0, k);
                let q = self.b.ins().ushr(x.1, k);
                let v = (self.vmask(p, n), self.vmask(q, n));
                self.vst(dst, v);
            }
            Inst::VIndex {
                signed,
                desc,
                w,
                adj,
                dst,
                idx,
            } => {
                let x = self.vld(idx);
                self.vknown(&[x.1], &mut sp, pc, inst);
                let k = match signed {
                    true => self.vsext(x.0, w),
                    false => x.0,
                };
                let v = match desc {
                    true => self.b.ins().iadd_imm_s(k, adj.wrapping_neg()),
                    false => {
                        let n = self.b.ins().ineg(k);
                        self.b.ins().iadd_imm_s(n, adj)
                    }
                };
                self.st(dst, v);
            }
            Inst::VTruth { dst, a, .. } => {
                let x = self.vld(a);
                let t = self.vtruth(x);
                self.st(dst, t);
            }
            Inst::VCase {
                kind, dst, a, b, ..
            } => {
                let (x, y) = (self.vld(a), self.vld(b));
                let d1 = self.b.ins().bxor(x.0, y.0);
                let d2 = self.b.ins().bxor(x.1, y.1);
                let diff = self.b.ins().bor(d1, d2);
                let diff = match kind {
                    VCaseKind::Case => diff,
                    _ => {
                        // z bits on either side match anything, and x bits for `casex`.
                        let z1 = self.b.ins().band_not(x.1, x.0);
                        let z2 = self.b.ins().band_not(y.1, y.0);
                        let mut care = self.b.ins().bor(z1, z2);
                        if kind == VCaseKind::Casex {
                            let x1 = self.b.ins().band(x.0, x.1);
                            let x2 = self.b.ins().band(y.0, y.1);
                            let xs = self.b.ins().bor(x1, x2);
                            care = self.b.ins().bor(care, xs);
                        }
                        self.b.ins().band_not(diff, care)
                    }
                };
                let m = self.b.ins().icmp_imm_s(IntCC::Equal, diff, 0);
                let m = self.b.ins().uextend(I64, m);
                self.st(dst, m);
            }
            Inst::VCount { signed, w, dst, a } => {
                let x = self.vld(a);
                self.vknown(&[x.1], &mut sp, pc, inst);
                let v = match signed {
                    true => {
                        let s = self.vsext(x.0, w);
                        let z = self.c64(0);
                        self.b.ins().smax(s, z)
                    }
                    false => x.0,
                };
                self.st(dst, v);
            }
            Inst::VRBin { op, dst, a, b } => {
                let (x, y) = (self.ldf(a), self.ldf(b));
                let v = match op {
                    FOp::Add => self.b.ins().fadd(x, y),
                    FOp::Sub => self.b.ins().fsub(x, y),
                    FOp::Mul => self.b.ins().fmul(x, y),
                    _ => self.b.ins().fdiv(x, y),
                };
                self.stf(dst, v);
            }
            Inst::VBack { cnt } => {
                let n = self.ld(cnt);
                let n = self.b.ins().iadd_imm_s(n, 1);
                // At the limit the interpreter counts the jump and reports the loop.
                let bad = self.b.ins().icmp_imm_s(
                    IntCC::UnsignedGreaterThan,
                    n,
                    yavs_interp::VERILOG_LOOP_LIMIT as i64,
                );
                self.vguard(bad, &mut sp, pc, inst);
                self.st(cnt, n);
            }
            _ => unreachable!("verilog_inline"),
        }
        self.vend(sp);
        true
    }

    /// A Verilog instruction not computed in place: the interpreter's own function for it
    /// (`yavs_rt_verilog`), the generic step only when that fails.
    pub(super) fn verilog_call(&mut self, pc: u32) {
        if self.nat.is_some() {
            self.bad = Some("a Verilog instruction on a native frame".into());
            return;
        }
        self.stepped.push("Verilog".into());
        let pcv = self.c32(pc);
        self.call_go(self.imp.verilog, &[pcv], true);
    }

    /// The join of the slow path, if a guard made one.
    fn vend(&mut self, sp: Option<Block>) {
        if sp.is_some()
            && let Some(j) = self.vjoin.take()
        {
            self.join(j);
        }
    }
}
