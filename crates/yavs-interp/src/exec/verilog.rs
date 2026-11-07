//! The Verilog instructions (`Inst::VUn` to `Inst::VBack`, #776): values of `yavs_logic::v4`
//! in runs of registers (`yavs_ir::vregs`), and the Verilog runtime they read nets from, write
//! nets to and register waits with ([`VerilogHost`]). The host is defined here and implemented
//! by `yavs-verilog`, which owns the scheduling domain; the machine holds it only while the
//! domain runs one of its processes (`InterpProcess::set_verilog_host`).

use yavs_ir::{FOp, Inst, Reg, VBinOp, VCaseKind, VEdge, VUnOp, vregs};
use yavs_kernel::{Ctx, SimError, Time};
use yavs_logic::v4::w64::{self, P};
use yavs_logic::v4::{Bit, V4, real_to_i64};

use super::{Fault, Machine, R, cold, ice, rt_err};

/// What a Verilog process reaches outside its registers. Net numbers are the running process's
/// own; the host maps them to its instance's nets.
pub trait VerilogHost: Send {
    /// The value of net `net` into `out` (the `a` plane words, then the `b` words, as many of
    /// each as half of `out`); its width.
    fn read(&mut self, net: u32, out: &mut [u64]) -> u32;
    /// Elements `at ..` of net `net` take the `n` bits of `planes` (`a` words, then `b`): at
    /// once (`nba` `None`), or in the NBA region `nba` femtoseconds from now. Elements outside
    /// the net are not written.
    fn write(
        &mut self,
        ctx: &mut Ctx<'_>,
        net: u32,
        at: i64,
        n: u32,
        planes: &[u64],
        nba: Option<u64>,
    ) -> Result<(), SimError>;
    /// The suspension that follows waits for `edge` of `net` too.
    fn wait_on(&mut self, net: u32, edge: VEdge);
    /// The suspension that follows waits `fs` femtoseconds (0: the Inactive region).
    fn wait_delay(&mut self, fs: u64);
    /// System task `id` with the argument registers' values.
    fn task(&mut self, ctx: &mut Ctx<'_>, id: u32, args: &[u64]) -> Result<(), SimError>;
    /// The running process, for messages.
    fn name(&self) -> &str;
    /// The host as the type that implements it (the domain takes it back after a run).
    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any + Send>;
}

/// Jumps back one activation of a Verilog process may take (`Inst::VBack`).
pub const LOOP_LIMIT: u64 = 1 << 26;

impl Machine {
    /// The value of `w` bits in the registers from `r`.
    fn vget(&self, r: Reg, w: u32) -> V4 {
        let n = (vregs(w) / 2) as usize;
        let at = self.regs.base + r.0 as usize;
        let v = &self.regs.v[at..at + 2 * n];
        V4::from_planes(w, &v[..n], &v[n..])
    }

    /// `v` into the registers from `r` (as many as its width takes).
    fn vput(&mut self, r: Reg, v: &V4) {
        let (a, b) = v.planes();
        let n = a.len();
        let at = self.regs.base + r.0 as usize;
        self.regs.v[at..at + n].copy_from_slice(a);
        self.regs.v[at + n..at + 2 * n].copy_from_slice(b);
    }

    fn host(&mut self) -> R<Box<dyn VerilogHost>> {
        self.vhost
            .take()
            .ok_or_else(|| cold(|| ice("a Verilog instruction outside the Verilog domain")))
    }

    /// One Verilog instruction.
    #[inline(never)]
    pub(super) fn verilog(&mut self, inst: Inst, ctx: &mut Option<&mut Ctx<'_>>) -> R<()> {
        if self.v64(inst, ctx)? {
            return Ok(());
        }
        match inst {
            Inst::VUn { op, w, dst, a } => {
                let a = self.vget(a, w);
                let v = match op {
                    VUnOp::Not => a.not(),
                    VUnOp::Neg => a.neg(),
                    VUnOp::LNot => a.lnot(),
                    VUnOp::RedAnd => a.red_and(),
                    VUnOp::RedNand => a.red_and().not(),
                    VUnOp::RedOr => a.red_or(),
                    VUnOp::RedNor => a.red_or().not(),
                    VUnOp::RedXor => a.red_xor(),
                    VUnOp::RedXnor => a.red_xor().not(),
                    VUnOp::Clog2 => a.clog2(),
                };
                self.vput(dst, &v);
            }
            Inst::VBin {
                op,
                signed: s,
                w,
                wb,
                dst,
                a,
                b,
            } => {
                let (a, b) = (self.vget(a, w), self.vget(b, wb));
                let v = match op {
                    VBinOp::Add => a.add(&b),
                    VBinOp::Sub => a.sub(&b),
                    VBinOp::Mul => a.mul(&b),
                    VBinOp::Div => a.div(&b, s),
                    VBinOp::Rem => a.rem(&b, s),
                    VBinOp::Pow(es) => a.pow(&b, s, es),
                    VBinOp::And => a.and(&b),
                    VBinOp::Or => a.or(&b),
                    VBinOp::Xor => a.xor(&b),
                    VBinOp::Xnor => a.xnor(&b),
                    VBinOp::Shl => a.shl(&b),
                    VBinOp::Shr => a.shr(&b, false),
                    VBinOp::Ashr => a.shr(&b, s),
                    VBinOp::Lt => a.lt(&b, s),
                    VBinOp::Le => a.le(&b, s),
                    VBinOp::Gt => a.gt(&b, s),
                    VBinOp::Ge => a.ge(&b, s),
                    VBinOp::Eq => a.eq(&b),
                    VBinOp::Ne => a.ne(&b),
                    VBinOp::CaseEq => a.case_eq(&b),
                    VBinOp::CaseNe => a.case_eq(&b).not(),
                    VBinOp::LAnd => a.land(&b),
                    VBinOp::LOr => a.lor(&b),
                };
                self.vput(dst, &v);
            }
            Inst::VExt {
                signed,
                w,
                wa,
                dst,
                a,
            } => {
                let v = self.vget(a, wa).ext(w, signed);
                self.vput(dst, &v);
            }
            Inst::VCat {
                whi,
                wlo,
                dst,
                hi,
                lo,
            } => {
                let v = V4::concat(&self.vget(hi, whi), &self.vget(lo, wlo));
                self.vput(dst, &v);
            }
            Inst::VMerge { w, dst, a, b } => {
                let v = self.vget(a, w).merge(&self.vget(b, w));
                self.vput(dst, &v);
            }
            Inst::VIndex {
                signed,
                desc,
                w,
                adj,
                dst,
                idx,
            } => {
                let at = self.vget(idx, w).to_i64(signed).and_then(|k| {
                    if desc {
                        k.checked_sub(adj)
                    } else {
                        adj.checked_sub(k)
                    }
                });
                self.set(dst, at.unwrap_or(i64::MIN) as u64);
            }
            Inst::VSel { w, n, dst, src, at } => {
                let v = self.vget(src, w).slice(self.ri(at), n);
                self.vput(dst, &v);
            }
            Inst::VTruth { w, dst, a } => {
                let t = match self.vget(a, w).truth() {
                    Bit::One => 1,
                    Bit::Zero => 0,
                    _ => 2,
                };
                self.set(dst, t);
            }
            Inst::VCase { kind, w, dst, a, b } => {
                let m = self.vget(a, w).case_match(
                    &self.vget(b, w),
                    kind != VCaseKind::Case,
                    kind == VCaseKind::Casex,
                );
                self.set(dst, m as u64);
            }
            Inst::VCount { signed, w, dst, a } => {
                // An unknown count runs no iteration (9.6).
                let n = self.vget(a, w).to_i64(signed).unwrap_or(0).max(0);
                self.set(dst, n as u64);
            }
            Inst::VDelay { unit, w, dst, a } => {
                let v = self.vget(a, w);
                // An x or z delay is zero (9.7.1).
                let fs = match (v.is_known(), v.to_u64()) {
                    (false, _) => 0,
                    (true, Some(u)) => u128::from(u) * u128::from(10u64.pow(unit.into())),
                    (true, None) => u128::MAX,
                };
                let fs = self.fs(fs)?;
                self.set(dst, fs);
            }
            Inst::VDelayR { unit, prec, dst, a } => {
                let r = self.rf(a);
                if r.is_nan() || r < 0.0 {
                    return self.verr(format!(
                        "the delay {r} in `{{}}` is negative (IEEE 1364-2005 9.7.1)"
                    ));
                }
                // Rounded to the precision of the module (19.8).
                let p = 10u64.pow(prec.into()) as f64;
                let u = 10u64.pow(unit.into()) as f64;
                let fs = self.fs(((r * u / p).round() * p) as u128)?;
                self.set(dst, fs);
            }
            Inst::VToReal { signed, w, dst, a } => {
                let r = self.vget(a, w).to_f64(signed);
                self.set(dst, r.to_bits());
            }
            Inst::VFromReal { w, dst, a } => {
                let v = V4::from_f64(w, self.rf(a));
                self.vput(dst, &v);
            }
            Inst::VRBin { op, dst, a, b } => {
                let (x, y) = (self.rf(a), self.rf(b));
                let r = match op {
                    FOp::Add => x + y,
                    FOp::Sub => x - y,
                    FOp::Mul => x * y,
                    FOp::Div => x / y,
                    FOp::Pow => x.powf(y),
                    FOp::PowI => x.powf(self.ri(b) as f64),
                };
                self.set(dst, r.to_bits());
            }
            Inst::VTime { unit, w, dst } => {
                let t = now(ctx)?;
                let u = 10u64.pow(unit.into());
                // Rounded to the module's unit (17.7.1).
                let v = V4::from_u64(64, (t + u / 2) / u).ext(w, false);
                self.vput(dst, &v);
            }
            Inst::VRTime { unit, dst } => {
                let r = now(ctx)? as f64 / 10u64.pow(unit.into()) as f64;
                self.set(dst, r.to_bits());
            }
            Inst::VRead { net, w, dst } => {
                let mut h = self.host()?;
                let at = self.regs.base + dst.0 as usize;
                let n = vregs(w) as usize;
                let got = h.read(net, &mut self.regs.v[at..at + n]);
                self.vhost = Some(h);
                if got != w {
                    return Err(cold(|| {
                        ice(format!("Verilog net {net} has {got} bits, read as {w}"))
                    }));
                }
            }
            Inst::VWrite { net, n, src, at } => {
                self.vwrite(ctx, net, self.ri(at), n, src, None)?;
            }
            Inst::VNba {
                net,
                n,
                src,
                at,
                delay,
            } => {
                let d = self.r(delay);
                self.vwrite(ctx, net, self.ri(at), n, src, Some(d))?;
            }
            Inst::VWaitOn { net, edge } => {
                let mut h = self.host()?;
                h.wait_on(net, edge);
                self.vhost = Some(h);
            }
            Inst::VWaitDelay { fs } => {
                let fs = self.r(fs);
                let mut h = self.host()?;
                h.wait_delay(fs);
                self.vhost = Some(h);
            }
            Inst::VTask { id, args } => {
                let f = self.rt.prog.func(self.frames[self.frames.len() - 1].func);
                let Some(regs) = f.and_then(|f| f.regs(args)) else {
                    return Err(cold(|| ice("a Verilog task without its argument list")));
                };
                let vals: Vec<u64> = regs.iter().map(|&r| self.r(r)).collect();
                let c = kernel(ctx)?;
                let mut h = self.host()?;
                let r = h.task(c, id, &vals);
                self.vhost = Some(h);
                r.map_err(Fault::Kernel)?;
            }
            Inst::VBack { cnt } => {
                let n = self.r(cnt) + 1;
                self.set(cnt, n);
                if n > LOOP_LIMIT {
                    return self.verr(format!(
                        "the Verilog process `{{}}` looped {LOOP_LIMIT} times without waiting"
                    ));
                }
            }
            _ => return Err(cold(|| ice("not a Verilog instruction"))),
        }
        Ok(())
    }

    fn vp(&self, r: Reg) -> P {
        (self.r(r), self.r(Reg(r.0 + 1)))
    }

    fn vput64(&mut self, r: Reg, x: P) {
        self.set(r, x.0);
        self.set(Reg(r.0 + 1), x.1);
    }

    /// The word forms of values of up to 64 bits (`w64`): `true` when done, `false` when the
    /// instruction takes the general path (a wider value, a case the word form leaves).
    #[inline(always)]
    fn v64(&mut self, inst: Inst, ctx: &mut Option<&mut Ctx<'_>>) -> R<bool> {
        let done = match inst {
            Inst::VUn { op, w, dst, a } if w <= 64 => match w64::un(un_code(op), w, self.vp(a)) {
                Some(v) => {
                    self.vput64(dst, v);
                    true
                }
                None => false,
            },
            Inst::VBin {
                op,
                signed,
                w,
                wb,
                dst,
                a,
                b,
            } if w <= 64 && wb <= 64 => {
                let (c, es) = bin_code(op);
                match w64::bin(c, signed, es, w, wb, self.vp(a), self.vp(b)) {
                    Some(v) => {
                        self.vput64(dst, v);
                        true
                    }
                    None => false,
                }
            }
            Inst::VExt {
                signed,
                w,
                wa,
                dst,
                a,
            } if w <= 64 && wa <= 64 => {
                let v = w64::ext(w, wa, signed, self.vp(a));
                self.vput64(dst, v);
                true
            }
            Inst::VCat {
                whi,
                wlo,
                dst,
                hi,
                lo,
            } if whi + wlo <= 64 => {
                let v = w64::cat(wlo, self.vp(hi), self.vp(lo));
                self.vput64(dst, v);
                true
            }
            Inst::VMerge { w, dst, a, b } if w <= 64 => {
                let v = w64::merge(self.vp(a), self.vp(b));
                self.vput64(dst, v);
                true
            }
            Inst::VIndex {
                signed,
                desc,
                w,
                adj,
                dst,
                idx,
            } if w <= 64 => {
                let x = self.vp(idx);
                let k = match (x.1, signed) {
                    (0, true) => Some(((x.0 << (64 - w)) as i64) >> (64 - w)),
                    (0, false) => i64::try_from(x.0).ok(),
                    _ => None,
                };
                let at = k.and_then(|k| {
                    if desc {
                        k.checked_sub(adj)
                    } else {
                        adj.checked_sub(k)
                    }
                });
                self.set(dst, at.unwrap_or(i64::MIN) as u64);
                true
            }
            Inst::VSel { w, n, dst, src, at } if w <= 64 && n <= 64 => {
                let v = w64::sel(w, n, self.vp(src), self.ri(at));
                self.vput64(dst, v);
                true
            }
            Inst::VTruth { w, dst, a } if w <= 64 => {
                let t = w64::truth(self.vp(a));
                self.set(dst, t);
                true
            }
            Inst::VCase { kind, w, dst, a, b } if w <= 64 => {
                let m = w64::case_match(
                    kind != VCaseKind::Case,
                    kind == VCaseKind::Casex,
                    self.vp(a),
                    self.vp(b),
                );
                self.set(dst, m as u64);
                true
            }
            Inst::VCount { signed, w, dst, a } if w <= 64 => {
                let n = w64::count(w, signed, self.vp(a));
                self.set(dst, n as u64);
                true
            }
            Inst::VDelay { unit, w, dst, a } if w <= 64 => {
                let x = self.vp(a);
                // An x or z delay is zero (9.7.1).
                let fs = match x.1 {
                    0 => u128::from(x.0) * u128::from(10u64.pow(unit.into())),
                    _ => 0,
                };
                let fs = self.fs(fs)?;
                self.set(dst, fs);
                true
            }
            Inst::VToReal { signed, w, dst, a } if w <= 64 => {
                match w64::to_f64(w, signed, self.vp(a)) {
                    Some(r) => {
                        self.set(dst, r.to_bits());
                        true
                    }
                    None => false,
                }
            }
            Inst::VFromReal { w, dst, a } if w <= 64 => {
                let v = real_to_i64(self.rf(a)) as u64 & w64::mask(w);
                self.vput64(dst, (v, 0));
                true
            }
            Inst::VTime { unit, w, dst } if w <= 64 => {
                let t = now(ctx)?;
                let u = 10u64.pow(unit.into());
                self.vput64(dst, (((t + u / 2) / u) & w64::mask(w), 0));
                true
            }
            _ => false,
        };
        Ok(done)
    }

    fn vwrite(
        &mut self,
        ctx: &mut Option<&mut Ctx<'_>>,
        net: u32,
        at: i64,
        n: u32,
        src: Reg,
        nba: Option<u64>,
    ) -> R<()> {
        let c = kernel(ctx)?;
        let mut h = self.host()?;
        let base = self.regs.base + src.0 as usize;
        let r = h.write(
            c,
            net,
            at,
            n,
            &self.regs.v[base..base + vregs(n) as usize],
            nba,
        );
        self.vhost = Some(h);
        r.map_err(Fault::Kernel)
    }

    /// Femtoseconds of a delay, or the error of one beyond the largest time.
    fn fs(&self, fs: u128) -> R<u64> {
        match u64::try_from(fs).ok().filter(|&f| Time::fs(f).is_some()) {
            Some(f) => Ok(f),
            None => self.verr("the delay in `{}` is beyond the largest time".into()),
        }
    }

    /// A runtime error whose message names the running process at its `{}`.
    fn verr<T>(&self, msg: String) -> R<T> {
        let name = self.vhost.as_ref().map_or("?", |h| h.name());
        rt_err(msg.replacen("{}", name, 1))
    }

    /// The host this machine holds, for the domain to take back.
    pub(crate) fn take_vhost(&mut self) -> Option<Box<dyn VerilogHost>> {
        self.vhost.take()
    }
}

fn kernel<'a, 'c>(ctx: &'a mut Option<&mut Ctx<'c>>) -> R<&'a mut Ctx<'c>> {
    match ctx.as_deref_mut() {
        Some(c) => Ok(c),
        None => rt_err("a Verilog instruction during elaboration"),
    }
}

fn now(ctx: &mut Option<&mut Ctx<'_>>) -> R<u64> {
    Ok(kernel(ctx)?.now().as_fs())
}

/// `VUnOp` as `w64::un` numbers it.
fn un_code(op: VUnOp) -> u8 {
    match op {
        VUnOp::Not => 0,
        VUnOp::Neg => 1,
        VUnOp::LNot => 2,
        VUnOp::RedAnd => 3,
        VUnOp::RedNand => 4,
        VUnOp::RedOr => 5,
        VUnOp::RedNor => 6,
        VUnOp::RedXor => 7,
        VUnOp::RedXnor => 8,
        VUnOp::Clog2 => 9,
    }
}

/// `VBinOp` as `w64::bin` numbers it, and the sign of a `**` exponent.
fn bin_code(op: VBinOp) -> (u8, bool) {
    use VBinOp::*;
    match op {
        Add => (0, false),
        Sub => (1, false),
        Mul => (2, false),
        Div => (3, false),
        Rem => (4, false),
        Pow(es) => (5, es),
        And => (6, false),
        Or => (7, false),
        Xor => (8, false),
        Xnor => (9, false),
        Shl => (10, false),
        Shr => (11, false),
        Ashr => (12, false),
        Lt => (13, false),
        Le => (14, false),
        Gt => (15, false),
        Ge => (16, false),
        Eq => (17, false),
        Ne => (18, false),
        CaseEq => (19, false),
        CaseNe => (20, false),
        LAnd => (21, false),
        LOr => (22, false),
    }
}
