//! Result tables for pure functions of one small discrete argument (#722).
//!
//! A pure function of a character, an enumeration or a small integer type that picks its
//! result with a `case` or an `if` chain (a character to a nibble, a state to its name) has a
//! handful of possible calls. When such a function is lowered, [`tabulate`] runs its optimised
//! body once for every value of the argument on a small abstract machine and, for the values
//! that compute a result without reporting or failing, keeps that result in a constant: the
//! [`CaseTab`] in `Program::tabs`. The optimiser (`opt/inline.rs`) then replaces a call of the
//! function by a range test, a table read and, for any value the table does not hold, the
//! call as it was, in both tiers.
//!
//! Exactness: a value is tabulated only if the machine ran the whole body, which has only
//! instructions that compute from the argument, constants and the function's own memory
//! (no report, no native, no call, no signal or package data); an instruction it does not know
//! and every runtime error leave the value to the call. `YAVS_NO_CASETAB=1` turns the pass
//! off. The pass reads the optimised body, so `YAVS_NO_IROPT=1` also turns it off.

use std::collections::HashMap;

use crate::{
    AddrSpace, BoolOp, CmpOp, ConstData, ConstId, FuncFlags, FuncId, FuncKind, Inst, IntOp,
    ParamClass, Program, ProgramBuilder, Ty, TyId, Width,
};

/// Most values of the argument type that get a table.
const MAX_VALUES: i64 = 256;
/// Most instructions the machine runs for one value.
const MAX_STEPS: u32 = 20_000;
/// Most bytes of one memory object of the machine (a result of an array bigger than this is
/// not tabulated) and of the whole table.
const MAX_OBJ: u64 = 1 << 16;
const MAX_TABLE: usize = 1 << 20;
/// A function with a `case` of more arms than this, or with more instructions as lowered, is
/// not one the inliner takes anyway.
const MIN_ARMS: usize = 7;
const MIN_CODE: usize = 60;

/// The results of a function for the values `lo .. lo + n` of its argument.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub struct CaseTab {
    pub lo: i64,
    pub n: u32,
    /// The result of value `lo + k` is at `k * stride`: a scalar as the 8 bytes of its register,
    /// an array as the bytes of its elements.
    pub table: ConstId,
    pub stride: u32,
    /// `n` bytes, 1 where the table holds the result, 0 where the function must be called;
    /// `None` when it holds every one.
    pub valid: Option<ConstId>,
    /// An array result: the descriptor every result has (`stride` is the size of its data).
    pub desc: Option<ConstId>,
    /// The argument type is an enumeration: every value of its register is in the table's
    /// range, no test needed.
    pub whole: bool,
    /// The result is an address (a fat pair in temporary memory), else an integer.
    pub ptr: bool,
}

/// Is the pass on?
pub(crate) fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("YAVS_NO_CASETAB")
            .map(|v| v.is_empty() || v == "0")
            .unwrap_or(true)
    })
}

/// The values an argument of type `t` can take: first value and count, if there are few.
fn domain(prog: &Program, t: TyId) -> Option<(i64, u32)> {
    let (lo, n) = match prog.types.get(t)? {
        Ty::Enum { lits, .. } => (0, lits.len() as i64),
        Ty::Int { lo, hi } => (*lo, hi.checked_sub(*lo)?.checked_add(1)?),
        _ => return None,
    };
    (1..=MAX_VALUES).contains(&n).then_some((lo, n as u32))
}

/// Table the function `id` if it is one the pass takes and the machine can run it (see the
/// module).
pub(crate) fn tabulate(pb: &mut ProgramBuilder, id: FuncId) {
    if !enabled() {
        return;
    }
    let prog = &pb.prog;
    let Some(f) = prog.funcs.get(id) else { return };
    let [p] = &f.params[..] else { return };
    let Some(ret) = f.ret_ty else { return };
    if f.kind != FuncKind::Function
        || !f.flags.contains(FuncFlags::PURE)
        || [
            FuncFlags::MAY_SUSPEND,
            FuncFlags::TOUCHES_SHARED,
            FuncFlags::TOUCHES_FILES,
            FuncFlags::HAS_COVER,
        ]
        .iter()
        .any(|&x| f.flags.contains(x))
        || f.parent.is_some()
        || f.nested
        || p.width != Width::B8
        || p.class != ParamClass::Value
        || p.mode != crate::Mode::In
        || p.arr.is_some()
    {
        return;
    }
    let Some((lo, n)) = p.ty.and_then(|t| domain(prog, t)) else {
        return;
    };
    let whole =
        p.ty.is_some_and(|t| matches!(prog.types.get(t), Some(Ty::Enum { .. })));
    let arr = match prog.types.get(ret) {
        Some(Ty::Int { .. } | Ty::Enum { .. } | Ty::Physical { .. }) => None,
        _ => match crate::memo_fn::plain_array(prog, ret) {
            Some(a) => Some(a),
            None => return,
        },
    };
    // Cheap test on the lowered code: no call, and a body the inliner would not take.
    let code = &f.full().code;
    let arms = code
        .iter()
        .filter_map(|i| match i {
            Inst::Case { table, .. } => prog.cases.get(*table).map(|t| t.arms.len()),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    if code.iter().any(|i| {
        matches!(
            i,
            Inst::Call { .. }
                | Inst::CallNative { .. }
                | Inst::CallForeign { .. }
                | Inst::CallIndirect { .. }
                | Inst::CallProt { .. }
        )
    }) || (arms < MIN_ARMS && code.len() < MIN_CODE)
    {
        return;
    }
    let Some(body) = crate::opt::optimize_body(prog, id) else {
        return;
    };
    // The value of each argument: the result bytes (scalar: the register), or `None`.
    let mut results: Vec<Option<Vec<u8>>> = Vec::with_capacity(n as usize);
    let mut shape: Option<Vec<u8>> = None;
    let mut ptr = false;
    for k in 0..i64::from(n) {
        let mut m = Machine::new(prog, &body, p.slot);
        let r = m.run(lo + k).and_then(|r| m.result(r, arr));
        results.push(match r {
            Some((bytes, desc)) => {
                ptr = desc.is_some();
                // Every result of an array has the descriptor of the first.
                match (&shape, desc) {
                    (None, d) => {
                        shape = d;
                        Some(bytes)
                    }
                    (Some(s), Some(d)) if *s == d => Some(bytes),
                    _ => None,
                }
            }
            None => None,
        });
    }
    let stride = match results.iter().flatten().next() {
        Some(r) if !r.is_empty() => r.len(),
        _ => return,
    };
    let good = results
        .iter()
        .filter(|r| r.as_ref().is_some_and(|r| r.len() == stride))
        .count();
    // Mostly misses: the call stays.
    if good * 2 < results.len() || stride * results.len() > MAX_TABLE {
        return;
    }
    let mut table = Vec::with_capacity(stride * results.len());
    let mut valid = Vec::with_capacity(results.len());
    for r in &results {
        match r {
            Some(r) if r.len() == stride => {
                table.extend_from_slice(r);
                valid.push(1u8);
            }
            _ => {
                table.resize(table.len() + stride, 0);
                valid.push(0);
            }
        }
    }
    let all = valid.iter().all(|&v| v == 1);
    let pb_prog = &mut pb.prog;
    let table = pb_prog.consts.push(ConstData::Bytes(table));
    let valid = (!all).then(|| pb_prog.consts.push(ConstData::Bytes(valid)));
    let desc = shape.map(|d| pb_prog.consts.push(ConstData::Bytes(d)));
    pb_prog.tabs.insert(
        id.0,
        CaseTab {
            lo,
            n,
            table,
            stride: stride as u32,
            valid,
            desc,
            whole,
            ptr,
        },
    );
}

/// A memory object of the machine.
struct Obj {
    bytes: Vec<u8>,
    ro: bool,
}

/// The abstract machine: the registers of the body, its frame, the constants it reads and the
/// temporary objects it makes. An address is `(object + 1) << 32 | offset`.
struct Machine<'a> {
    prog: &'a Program,
    f: &'a crate::Func,
    regs: Vec<u64>,
    mem: Vec<Obj>,
    consts: HashMap<ConstId, usize>,
}

type W<T> = Option<T>;

fn at(obj: usize, off: u64) -> u64 {
    ((obj as u64 + 1) << 32) | off
}

/// The bytes of a constant, as the interpreter expands it.
fn expand(c: &ConstData) -> W<Vec<u8>> {
    Some(match c {
        ConstData::Bytes(b) => b.clone(),
        ConstData::Repeat { .. } | ConstData::Runs(_) => {
            let total = match c {
                ConstData::Repeat { elem, n } => (elem.len() as u64).checked_mul(*n)?,
                ConstData::Runs(r) => r.iter().try_fold(0u64, |t, (n, e)| {
                    t.checked_add((e.len() as u64).checked_mul(*n)?)
                })?,
                _ => 0,
            };
            if total > MAX_OBJ {
                return None;
            }
            let mut out = vec![0; total as usize];
            c.read(0, &mut out).then_some(out)?
        }
        ConstData::Logic(runs) => {
            let total: u64 = runs.iter().map(|r| r.0).sum();
            if total > MAX_OBJ {
                return None;
            }
            let mut out = vec![0u8; total.div_ceil(64) as usize * 32];
            let mut e = 0u64;
            for &(n, code) in runs {
                if code > 8 {
                    return None;
                }
                for _ in 0..n {
                    for p in 0..4 {
                        if code >> p & 1 == 1 {
                            out[(e / 64) as usize * 32 + p * 8 + (e % 64 / 8) as usize] |=
                                1 << (e % 8);
                        }
                    }
                    e += 1;
                }
            }
            out
        }
    })
}

impl<'a> Machine<'a> {
    fn new(prog: &'a Program, f: &'a crate::Func, _slot: u32) -> Self {
        Machine {
            prog,
            f,
            regs: vec![0; f.reg_ty.len()],
            mem: Vec::new(),
            consts: HashMap::new(),
        }
    }

    fn r(&self, r: crate::Reg) -> W<u64> {
        self.regs.get(r.0 as usize).copied()
    }

    fn set(&mut self, r: crate::Reg, v: u64) -> W<()> {
        *self.regs.get_mut(r.0 as usize)? = v;
        Some(())
    }

    fn new_obj(&mut self, bytes: Vec<u8>, ro: bool) -> W<usize> {
        (bytes.len() as u64 <= MAX_OBJ).then_some(())?;
        self.mem.push(Obj { bytes, ro });
        Some(self.mem.len() - 1)
    }

    fn const_obj(&mut self, c: ConstId) -> W<usize> {
        if let Some(&o) = self.consts.get(&c) {
            return Some(o);
        }
        let bytes = expand(self.prog.consts.get(c)?)?;
        let o = self.new_obj(bytes, true)?;
        self.consts.insert(c, o);
        Some(o)
    }

    fn split(a: u64) -> W<(usize, usize)> {
        Some((
            (a >> 32).checked_sub(1)? as usize,
            (a & 0xffff_ffff) as usize,
        ))
    }

    fn rd(&self, a: u64, n: u64) -> W<&[u8]> {
        let (o, off) = Self::split(a)?;
        self.mem
            .get(o)?
            .bytes
            .get(off..off.checked_add(n as usize)?)
    }

    fn wr(&mut self, a: u64, n: u64) -> W<&mut [u8]> {
        let (o, off) = Self::split(a)?;
        let obj = self.mem.get_mut(o)?;
        if obj.ro {
            return None;
        }
        obj.bytes.get_mut(off..off.checked_add(n as usize)?)
    }

    fn load(&self, a: u64, w: Width) -> W<u64> {
        Some(match w {
            Width::B1 => u64::from(self.rd(a, 1)?[0]),
            Width::B4 => i64::from(i32::from_le_bytes(self.rd(a, 4)?.try_into().ok()?)) as u64,
            Width::B8 => u64::from_le_bytes(self.rd(a, 8)?.try_into().ok()?),
            Width::Sig => return None,
        })
    }

    fn store(&mut self, a: u64, v: u64, w: Width) -> W<()> {
        let n = match w {
            Width::B1 => 1,
            Width::B4 => 4,
            Width::B8 => 8,
            Width::Sig => return None,
        };
        self.wr(a, n)?
            .copy_from_slice(&v.to_le_bytes()[..n as usize]);
        Some(())
    }

    /// Run the body with argument `v`; the value it returns.
    fn run(&mut self, v: i64) -> W<u64> {
        let slot = self.f.params.first()?.slot as usize;
        let mut frame = vec![0u8; (self.f.frame as usize).max(slot + 8)];
        frame[slot..slot + 8].copy_from_slice(&v.to_le_bytes());
        self.new_obj(frame, false)?;
        // A register defined once by a constant or an address starts with it, wherever the
        // definition is (the optimiser relies on frames starting so).
        let mut defs = vec![0u32; self.regs.len()];
        for i in &self.f.code {
            i.visit(&mut |r, role, _| {
                if role == crate::Role::Def
                    && let Some(d) = defs.get_mut(r.0 as usize)
                {
                    *d += 1;
                }
            });
        }
        for i in self.f.code.iter().copied() {
            if let Inst::Const { dst, .. } | Inst::AddrOf { dst, .. } = i
                && defs.get(dst.0 as usize) == Some(&1)
            {
                self.step(i)?;
            }
        }
        let mut b = crate::BlockId(0);
        let mut steps = 0;
        loop {
            let blk = *self.f.blocks.get(b)?;
            let mut next = None;
            for pc in blk.start as usize..blk.end as usize {
                steps += 1;
                if steps > MAX_STEPS {
                    return None;
                }
                match *self.f.code.get(pc)? {
                    Inst::Br { to } => next = Some(to),
                    Inst::CondBr { c, t, f } => next = Some(if self.r(c)? != 0 { t } else { f }),
                    Inst::Case { v, table } => {
                        let t = self.prog.cases.get(table)?;
                        let x = self.r(v)? as i64;
                        next = Some(
                            t.arms
                                .iter()
                                .find(|&&(lo, hi, _)| lo <= x && x <= hi)
                                .map_or(t.default, |a| a.2),
                        );
                    }
                    Inst::Ret { v } => return self.r(v?),
                    i => self.step(i)?,
                }
            }
            b = next?;
        }
    }

    /// One instruction that does not branch.
    fn step(&mut self, i: Inst) -> W<()> {
        match i {
            Inst::Stmt | Inst::TempRoot { .. } => {}
            Inst::Const { dst, v } => self.set(dst, v as u64)?,
            Inst::Move { dst, src } => self.set(dst, self.r(src)?)?,
            Inst::Select { dst, c, a, b } => {
                let v = if self.r(c)? != 0 {
                    self.r(a)?
                } else {
                    self.r(b)?
                };
                self.set(dst, v)?;
            }
            Inst::AddrOf { dst, space, off } => {
                let a = match space {
                    AddrSpace::Frame => at(0, u64::from(off)),
                    AddrSpace::Const(c) => at(self.const_obj(c)?, u64::from(off)),
                    _ => return None,
                };
                self.set(dst, a)?;
            }
            Inst::Load { dst, addr, w } => {
                let v = self.load(self.r(addr)?, w)?;
                self.set(dst, v)?;
            }
            Inst::Store { addr, src, w } => self.store(self.r(addr)?, self.r(src)?, w)?,
            Inst::Index {
                dst,
                base,
                pos,
                stride,
            } => {
                let (o, off) = Self::split(self.r(base)?)?;
                let off = (off as i64)
                    .checked_add((self.r(pos)? as i64).checked_mul(i64::from(stride))?)?;
                let off = u64::try_from(off)
                    .ok()
                    .filter(|&o| o <= u64::from(u32::MAX))?;
                self.set(dst, at(o, off))?;
            }
            Inst::Alloc { dst, size } => {
                let o = self.new_obj(vec![0; self.r(size)?.min(MAX_OBJ + 1) as usize], false)?;
                self.set(dst, at(o, 0))?;
            }
            Inst::NewFrame {
                dst,
                desc,
                dsize,
                data,
                size,
                fill,
            } => {
                let size = self.r(size)?;
                let ds = u64::from(dsize);
                if size > MAX_OBJ || (fill != 0 && size % u64::from(fill) != 0) {
                    return None;
                }
                let mut bytes = vec![0u8; (ds + size) as usize];
                if ds != 0 {
                    bytes[..ds as usize].copy_from_slice(self.rd(self.r(desc)?, ds)?);
                }
                let val = &mut bytes[ds as usize..];
                if fill == 0 {
                    val.copy_from_slice(self.rd(self.r(data)?, size)?);
                } else if size != 0 {
                    let pat = self.rd(self.r(data)?, u64::from(fill))?;
                    for c in val.chunks_exact_mut(fill as usize) {
                        c.copy_from_slice(pat);
                    }
                }
                let o = self.new_obj(bytes, false)?;
                self.set(dst, at(o, 0))?;
            }
            Inst::Copy { dst, src, bytes } => {
                let n = self.r(bytes)?;
                let data = self.rd(self.r(src)?, n)?.to_vec();
                self.wr(self.r(dst)?, n)?.copy_from_slice(&data);
            }
            Inst::LCopy {
                dst,
                dst_off,
                src,
                src_off,
                n,
            } => {
                let (d, ds, s, ss, n) = (
                    self.r(dst)?,
                    self.r(dst_off)?,
                    self.r(src)?,
                    self.r(src_off)?,
                    self.r(n)?,
                );
                if n > MAX_OBJ {
                    return None;
                }
                // Bits of the four planes of each 32-byte block (the order of `LCopy` does not
                // matter: the ranges of a source and a destination in one object do not overlap
                // in the code that is tabulated, and a copy that did would return `None` below).
                let (so, sb) = Self::split(s)?;
                let (dobj, db) = Self::split(d)?;
                if so == dobj {
                    return None;
                }
                for e in 0..n {
                    let (se, de) = (ss.checked_add(e)?, ds.checked_add(e)?);
                    let sbyte = sb + (se / 64) as usize * 32 + (se % 64 / 8) as usize;
                    let dbyte = db + (de / 64) as usize * 32 + (de % 64 / 8) as usize;
                    for p in 0..4 {
                        let bit = self.mem.get(so)?.bytes.get(sbyte + p * 8)? >> (se % 8) & 1;
                        let o = self.mem.get_mut(dobj)?;
                        if o.ro {
                            return None;
                        }
                        let byte = o.bytes.get_mut(dbyte + p * 8)?;
                        *byte = *byte & !(1 << (de % 8)) | bit << (de % 8);
                    }
                }
            }
            Inst::DopeLen { dst, dope, dim } => {
                let d = self.rd(self.r(dope)? + u64::from(dim) * 24, 24)?;
                let w =
                    |k: usize| i64::from_le_bytes(d[k * 8..k * 8 + 8].try_into().unwrap_or([0; 8]));
                self.set(dst, crate::dim_len(w(0), w(1), w(2) == 1)?)?;
            }
            Inst::Cmp { op, dst, a, b } => {
                let o = (self.r(a)? as i64).cmp(&(self.r(b)? as i64));
                let v = match op {
                    CmpOp::Eq => o.is_eq(),
                    CmpOp::Ne => o.is_ne(),
                    CmpOp::Lt => o.is_lt(),
                    CmpOp::Le => o.is_le(),
                    CmpOp::Gt => o.is_gt(),
                    CmpOp::Ge => o.is_ge(),
                };
                self.set(dst, v.into())?;
            }
            Inst::Bool { op, dst, a, b } => {
                let (x, y) = (self.r(a)? != 0, self.r(b)? != 0);
                let v = match op {
                    BoolOp::And => x && y,
                    BoolOp::Or => x || y,
                    BoolOp::Xor => x != y,
                    BoolOp::Not => !x,
                };
                self.set(dst, v.into())?;
            }
            Inst::Bin { op, dst, a, b } => {
                let (x, y) = (self.r(a)? as i64, self.r(b)? as i64);
                let v = match op {
                    IntOp::Add => x.checked_add(y)?,
                    IntOp::Sub => x.checked_sub(y)?,
                    IntOp::Mul => x.checked_mul(y)?,
                    _ => return None,
                };
                self.set(dst, v as u64)?;
            }
            Inst::BinRange {
                op,
                dst,
                a,
                b,
                lo,
                hi,
                ..
            } => {
                let (x, y) = (self.r(a)? as i64, self.r(b)? as i64);
                let v = match op {
                    IntOp::Add => x.checked_add(y)?,
                    IntOp::Sub => x.checked_sub(y)?,
                    IntOp::Mul => x.checked_mul(y)?,
                    _ => return None,
                };
                (self.r(lo)? as i64 <= v && v <= self.r(hi)? as i64).then_some(())?;
                self.set(dst, v as u64)?;
            }
            Inst::Check {
                kind: crate::CheckKind::Range,
                v,
                lo,
                hi,
                ..
            } => {
                let v = self.r(v)? as i64;
                (self.r(lo)? as i64 <= v && v <= self.r(hi)? as i64).then_some(())?;
            }
            _ => return None,
        }
        Some(())
    }

    /// What a return of `r` is: for a scalar the 8 bytes of the register, for an array the
    /// bytes of its elements and of its descriptor.
    fn result(&self, r: u64, arr: Option<crate::MemoArr>) -> W<(Vec<u8>, Option<Vec<u8>>)> {
        let Some(a) = arr else {
            return Some((r.to_le_bytes().to_vec(), None));
        };
        let data = u64::from_le_bytes(self.rd(r, 8)?.try_into().ok()?);
        let dope = u64::from_le_bytes(self.rd(r + 8, 8)?.try_into().ok()?);
        let desc = self.rd(dope, a.desc_bytes() as u64)?;
        let n = a.data_bytes(desc)?;
        let bytes = self.rd(data, n)?;
        Some((bytes.to_vec(), Some(desc.to_vec())))
    }
}
