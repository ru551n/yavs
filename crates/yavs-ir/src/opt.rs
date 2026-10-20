//! Machine-independent optimisation of a function's code, run once per function when the
//! program is finished (after flag propagation, before resume points are numbered and the
//! verifier runs).
//!
//! Lowering emits every operand where it is used, so the code is full of constants and
//! trivial arithmetic on them. This pass walks the extended basic blocks of a function (a
//! block and the blocks only it can reach) and
//!
//! - folds constants and trivial arithmetic (`x * 1`, `x + 0`, `x * 0`) and branches on
//!   constants,
//! - turns an offset from a known address into a known address (`AddrOf`),
//! - drops pure definitions nobody reads.
//!
//! Invariants it keeps (ir.md 1.2): no register becomes live across a suspension (a
//! register is replaced by another only when the other is defined since the last
//! suspension on the path, or the function never suspends), every surviving instruction
//! keeps its statement, `Stmt` and `Cover` are never touched, a check is removed only when
//! it provably passes, and stores are never removed (debuggers read frame slots).
//! Functions of kind `Elab` and `Thunk` run once per instance and are left alone.
//! `YAVS_NO_IROPT=1` turns the pass off (differential testing).

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use crate::{
    AddrSpace, BlockId, BoolOp, CheckKind, CheckMsgId, CmpOp, ConstData, FuncFlags, FuncId,
    FuncKind, Inst, IntOp, IntUnOp, Program, Reg, RegTy, Role, StmtId, Width, dim_len,
};

/// Is the pass enabled? `YAVS_NO_IROPT` set to anything but `0` or the empty string turns
/// it off.
pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("YAVS_NO_IROPT")
            .map(|v| v.is_empty() || v == "0")
            .unwrap_or(true)
    })
}

/// Optimise the functions `from .. to` of `prog`. Flags must be propagated already.
pub(crate) fn optimize(prog: &mut Program, from: usize, to: usize) {
    if !enabled() {
        return;
    }
    let susp: Vec<bool> = prog
        .funcs
        .values()
        .iter()
        .map(|f| f.flags.contains(FuncFlags::MAY_SUSPEND))
        .collect();
    for i in from..to.min(prog.funcs.len()) {
        let id = FuncId(i as u32);
        let Some(f) = prog.funcs.get(id) else {
            continue;
        };
        if matches!(f.kind, FuncKind::Elab | FuncKind::Thunk) || f.code.is_empty() {
            continue;
        }
        let mut o = Opt {
            prog,
            susp: &susp,
            reg_ty: &f.reg_ty,
            code: f.code.clone(),
            operands: f.operands.clone(),
            blocks: f.blocks.values().iter().map(|b| (b.start, b.end)).collect(),
            body: f.body,
            frame: f.frame,
            named: f
                .locals
                .iter()
                .map(|l| (i64::from(l.slot), i64::from(l.size)))
                .chain(
                    f.params
                        .iter()
                        .map(|p| (i64::from(p.slot), i64::from(p.width.bytes()))),
                )
                .collect(),
            defs: Vec::new(),
            kc: HashMap::new(),
            ka: HashMap::new(),
            rep: HashMap::new(),
            nosusp: false,
            st: State::default(),
            changed: false,
        };
        o.run();
        if !o.changed {
            continue;
        }
        let (code, loc, blocks, operands) = o.finish(&f.loc);
        if let Some(f) = prog.funcs.get_mut(id) {
            for (i, nb) in blocks.iter().enumerate() {
                if let Some(b) = f.blocks.get_mut(BlockId(i as u32)) {
                    b.start = nb.0;
                    b.end = nb.1;
                }
            }
            f.code = code;
            f.loc = loc;
            f.operands = operands;
        }
    }
}

/// Marker for a deleted instruction until the code is compacted: an `Unreachable` with this
/// message never occurs otherwise.
const DEAD_MSG: CheckMsgId = CheckMsgId(u32::MAX);

fn dead() -> Inst {
    Inst::Unreachable { what: DEAD_MSG }
}

fn is_dead(i: &Inst) -> bool {
    matches!(i, Inst::Unreachable { what } if *what == DEAD_MSG)
}

/// A normalised operand: a known constant, a known address, or a register.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Opn {
    K(i64),
    A(AddrSpace, u32),
    R(Reg),
}

/// An available expression. Memory-dependent ones (loads, descriptor reads) are dropped when
/// something may write the memory they read.
#[derive(Clone, PartialEq, Eq, Hash)]
enum Key {
    Bin(IntOp, Opn, Opn),
    BinRange(IntOp, Opn, Opn, Opn, Opn, CheckMsgId),
    Un(IntUnOp, Opn),
    Cmp(CmpOp, Opn, Opn),
    Bool(BoolOp, Opn, Opn),
    Index(Opn, Opn, u32),
    Check(CheckKind, Opn, Opn, Opn, CheckMsgId),
    Load(Opn, Width),
    Dope(Opn, u8),
    Pos(Opn, u8, Opn, CheckMsgId),
    Field(Opn, Opn, u32),
}

impl Key {
    /// The address operand and the byte range read from it, for memory-dependent keys.
    fn region(&self) -> Option<(Opn, i64, i64)> {
        match *self {
            Key::Load(a, w) => Some((a, 0, i64::from(w.bytes()))),
            Key::Dope(a, d) | Key::Pos(a, d, ..) => Some((a, i64::from(d) * 24, 24)),
            Key::Field(_, l, n) => Some((l, 16 + 8 * i64::from(n), 8)),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
enum Val {
    R(Reg),
    K(i64),
    Unit,
}

/// What is known on the way through an extended basic block.
#[derive(Clone, Default)]
struct State {
    avail: HashMap<Key, Val>,
    /// Registers defined since the last suspension point.
    local: HashSet<Reg>,
}

/// What a simplified arithmetic instruction becomes.
enum Simp {
    Fold(i64),
    Same(Reg),
}

struct Opt<'a> {
    prog: &'a Program,
    susp: &'a [bool],
    reg_ty: &'a [RegTy],
    code: Vec<Inst>,
    operands: Vec<Reg>,
    blocks: Vec<(u32, u32)>,
    body: Option<BlockId>,
    frame: u32,
    /// Frame ranges a debugger shows and may write: named locals and parameters.
    named: Vec<(i64, i64)>,
    /// Definitions per register.
    defs: Vec<u32>,
    /// Registers defined once, by a `Const`.
    kc: HashMap<Reg, i64>,
    /// Registers defined once, by an `AddrOf`.
    ka: HashMap<Reg, (AddrSpace, u32)>,
    /// Registers replaced by another (both defined once).
    rep: HashMap<Reg, Reg>,
    /// The function has no suspension point.
    nosusp: bool,
    st: State,
    changed: bool,
}

impl Opt<'_> {
    fn is_susp(&self, i: &Inst) -> bool {
        match *i {
            Inst::Suspend { .. } => true,
            Inst::Call { func, .. } => self.susp.get(func.0 as usize) == Some(&true),
            _ => false,
        }
    }

    /// A register defined exactly once, holding a plain value.
    fn single(&self, r: Reg) -> bool {
        self.defs.get(r.0 as usize) == Some(&1)
            && matches!(self.reg_ty.get(r.0 as usize), Some(RegTy::I64 | RegTy::Ptr))
    }

    fn resolve(&self, mut r: Reg) -> Reg {
        while let Some(&x) = self.rep.get(&r) {
            r = x;
        }
        r
    }

    fn opn(&self, r: Reg) -> Opn {
        let r = self.resolve(r);
        if let Some(&v) = self.kc.get(&r) {
            Opn::K(v)
        } else if let Some(&(s, o)) = self.ka.get(&r) {
            Opn::A(s, o)
        } else {
            Opn::R(r)
        }
    }

    fn analyse(&mut self) {
        self.defs = vec![0; self.reg_ty.len()];
        self.nosusp = self.body.is_none();
        for pc in 0..self.code.len() {
            let i = self.code[pc];
            if self.is_susp(&i) {
                self.nosusp = false;
            }
            let defs = &mut self.defs;
            i.visit(&mut |r, role, _| {
                if role == Role::Def
                    && let Some(d) = defs.get_mut(r.0 as usize)
                {
                    *d += 1;
                }
            });
        }
        for pc in 0..self.code.len() {
            match self.code[pc] {
                Inst::Const { dst, v } if self.single(dst) => {
                    self.kc.insert(dst, v);
                }
                Inst::AddrOf { dst, space, off } if self.single(dst) => {
                    self.ka.insert(dst, (space, off));
                }
                _ => {}
            }
        }
    }

    fn run(&mut self) {
        for _ in 0..3 {
            self.rep.clear();
            self.kc.clear();
            self.ka.clear();
            self.analyse();
            if !self.round() {
                break;
            }
        }
    }

    /// One walk, a rename and a dead-code sweep; did anything change?
    fn round(&mut self) -> bool {
        let mut any = false;
        any |= self.walk_all();
        any |= self.apply_rep();
        any |= self.dce();
        self.changed |= any;
        any
    }

    fn range(&self, b: usize) -> std::ops::Range<usize> {
        self.blocks[b].0 as usize..self.blocks[b].1 as usize
    }

    /// Replace the operands of `pc` through `rep`.
    fn rewrite_uses(&mut self, pc: usize) -> bool {
        let mut inst = self.code[pc];
        let mut ch = false;
        let rep = &self.rep;
        let res = |mut r: Reg| {
            while let Some(&x) = rep.get(&r) {
                r = x;
            }
            r
        };
        inst.visit_mut(&mut |r, role, _| {
            if role == Role::Use {
                let n = res(*r);
                if n != *r {
                    *r = n;
                    ch = true;
                }
            }
        });
        if let Some(l) = inst.list() {
            let (s, e) = (l.start as usize, (l.start + l.len) as usize);
            for k in s..e {
                let n = res(self.operands[k]);
                if n != self.operands[k] {
                    self.operands[k] = n;
                    ch = true;
                }
            }
        }
        self.code[pc] = inst;
        ch
    }

    fn apply_rep(&mut self) -> bool {
        if self.rep.is_empty() {
            return false;
        }
        let mut ch = false;
        for pc in 0..self.code.len() {
            if !is_dead(&self.code[pc]) {
                ch |= self.rewrite_uses(pc);
            }
        }
        ch
    }

    /// The constant `dst` becomes.
    fn make_const(&mut self, pc: usize, dst: Reg, v: i64) {
        self.code[pc] = Inst::Const { dst, v };
        if self.single(dst) {
            self.kc.insert(dst, v);
        }
    }

    fn make_addr(&mut self, pc: usize, dst: Reg, space: AddrSpace, off: u32) {
        self.code[pc] = Inst::AddrOf { dst, space, off };
        if self.single(dst) {
            self.ka.insert(dst, (space, off));
        }
    }

    /// Drop `pc`, using `x` wherever `dst` was read. Only when both are defined once.
    fn alias(&mut self, pc: usize, dst: Reg, x: Reg) -> bool {
        if self.single(dst) && self.single(x) && dst != x {
            self.rep.insert(dst, x);
            self.code[pc] = dead();
            true
        } else {
            false
        }
    }

    /// Walk every extended basic block: a root (entry, process body, or a block with
    /// several predecessors) and, depth first, each block only its predecessor can reach.
    fn walk_all(&mut self) -> bool {
        let n = self.blocks.len();
        let mut preds = vec![0u32; n];
        let mut first = vec![usize::MAX; n];
        for b in 0..n {
            let Some(last) = self.range(b).last() else {
                continue;
            };
            let mut edge = |to: BlockId| {
                if let Some(p) = preds.get_mut(to.0 as usize) {
                    *p += 1;
                    first[to.0 as usize] = b;
                }
            };
            match self.code[last] {
                Inst::Br { to } => edge(to),
                Inst::CondBr { t, f, .. } => {
                    edge(t);
                    edge(f);
                }
                Inst::Case { table, .. } => {
                    if let Some(t) = self.prog.cases.get(table) {
                        edge(t.default);
                        for a in &t.arms {
                            edge(a.2);
                        }
                    }
                }
                _ => {}
            }
        }
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut roots = Vec::new();
        for b in 0..n {
            let root = b == 0
                || preds[b] != 1
                || first[b] == b
                || self.body.is_some_and(|x| x.0 as usize == b);
            if root {
                roots.push(b);
            } else {
                children[first[b]].push(b);
            }
        }
        let mut any = false;
        for r in roots {
            let mut stack = vec![(r, State::default())];
            while let Some((b, st)) = stack.pop() {
                self.st = st;
                any |= self.walk(b);
                let kids = &children[b];
                for (k, &c) in kids.iter().enumerate() {
                    let st = if k + 1 == kids.len() {
                        std::mem::take(&mut self.st)
                    } else {
                        self.st.clone()
                    };
                    stack.push((c, st));
                }
            }
        }
        any
    }

    fn walk(&mut self, b: usize) -> bool {
        let mut any = false;
        for pc in self.range(b) {
            if is_dead(&self.code[pc]) {
                continue;
            }
            any |= self.rewrite_uses(pc);
            any |= self.simplify(pc);
            let i = self.code[pc];
            if is_dead(&i) {
                continue;
            }
            if self.is_susp(&i) {
                self.st = State::default();
                continue;
            }
            self.effect(&i);
            let local = &mut self.st.local;
            i.visit(&mut |r, role, _| {
                if role == Role::Def {
                    local.insert(r);
                }
            });
        }
        any
    }

    /// Forget what the instruction may overwrite.
    fn effect(&mut self, i: &Inst) {
        match *i {
            Inst::Store { addr, src, w } => {
                let a = self.opn(addr);
                self.kill(a, i64::from(w.bytes()));
                // A stored 8-byte value is what a load of the address gives back.
                if w == Width::B8 && self.key_ok(a) {
                    let v = match self.opn(src) {
                        Opn::K(v) => Some(Val::K(v)),
                        Opn::R(r) if self.single(r) => Some(Val::R(r)),
                        _ => None,
                    };
                    if let Some(v) = v {
                        self.st.avail.insert(Key::Load(a, Width::B8), v);
                    }
                }
            }
            // A debugger stopped here may have written a variable: forget what is known of
            // named locals and of memory reached through pointers; hidden frame slots stay.
            Inst::Stmt => {
                let named = &self.named;
                self.st.avail.retain(|k, _| match k.region() {
                    None => true,
                    Some((Opn::A(AddrSpace::Const(_), _), ..)) => true,
                    Some((Opn::A(AddrSpace::Frame, o), lo, n)) => {
                        let a = i64::from(o) + lo;
                        named.iter().all(|&(s, z)| a + n <= s || s + z <= a)
                    }
                    Some(_) => false,
                });
            }
            Inst::Const { .. }
            | Inst::ConstF { .. }
            | Inst::Move { .. }
            | Inst::AddrOf { .. }
            | Inst::Load { .. }
            | Inst::Alloc { .. }
            | Inst::PkgAlloc { .. }
            | Inst::Index { .. }
            | Inst::Field { .. }
            | Inst::Bin { .. }
            | Inst::Un { .. }
            | Inst::FBin { .. }
            | Inst::FUn { .. }
            | Inst::Phys { .. }
            | Inst::IToF { .. }
            | Inst::FToI { .. }
            | Inst::FToIRange { .. }
            | Inst::Cmp { .. }
            | Inst::FCmp { .. }
            | Inst::Bool { .. }
            | Inst::LEq { .. }
            | Inst::LGet { .. }
            | Inst::Check { .. }
            | Inst::FCheck { .. }
            | Inst::BinRange { .. }
            | Inst::Pos { .. }
            | Inst::DopeLen { .. }
            | Inst::SubtypeCheck { .. }
            | Inst::Br { .. }
            | Inst::CondBr { .. }
            | Inst::Case { .. }
            | Inst::Ret { .. }
            | Inst::Unreachable { .. }
            | Inst::Deref { .. }
            | Inst::New { .. }
            | Inst::SigVal { .. }
            | Inst::SigParts { .. }
            | Inst::SigSlice { .. }
            | Inst::SigReadScalar { .. }
            | Inst::SigAttr { .. }
            | Inst::Driving { .. }
            | Inst::Drive { .. }
            | Inst::DriveSlot { .. }
            | Inst::Disconnect { .. }
            | Inst::Force { .. }
            | Inst::Release { .. }
            | Inst::WaitOn { .. }
            | Inst::WaitFor { .. }
            | Inst::WaitAt { .. }
            | Inst::TimedOut { .. }
            | Inst::Now { .. }
            | Inst::Delta { .. }
            | Inst::Report { .. }
            | Inst::Output { .. }
            | Inst::Cover { .. } => {}
            // Stores through a pointer, calls, statement boundaries (a debugger may write a
            // variable): anything may have changed.
            _ => self.kill_all(),
        }
    }

    fn kill_all(&mut self) {
        self.st.avail.retain(|k, _| match k.region() {
            None => true,
            Some((a, ..)) => matches!(a, Opn::A(AddrSpace::Const(_), _)),
        });
    }

    /// A store of `len` bytes at `at`.
    fn kill(&mut self, at: Opn, len: i64) {
        self.st.avail.retain(|k, _| match k.region() {
            None => true,
            Some((Opn::A(AddrSpace::Const(_), _), ..)) => true,
            Some((Opn::A(s2, o2), lo, n)) => match at {
                Opn::A(s1, _) if s1 != s2 => true,
                Opn::A(_, o1) => {
                    let (a, b) = (i64::from(o1), i64::from(o2) + lo);
                    !(a + len <= b || b + n <= a)
                }
                _ => false,
            },
            Some(_) => false,
        });
    }

    /// May the address be used as part of a key (not a register redefined later)?
    fn key_ok(&self, a: Opn) -> bool {
        match a {
            Opn::R(r) => self.single(r),
            _ => true,
        }
    }

    fn kopn(&self, r: Reg) -> Option<Opn> {
        let o = self.opn(r);
        self.key_ok(o).then_some(o)
    }

    /// Reuse an available value, or record this one.
    fn cse(&mut self, pc: usize, dst: Option<Reg>, key: Key) -> bool {
        if let Some(v) = self.st.avail.get(&key).copied() {
            match (dst, v) {
                (None, Val::Unit) => {
                    self.code[pc] = dead();
                    return true;
                }
                (Some(d), Val::R(x)) if self.alias(pc, d, x) => return true,
                (Some(d), Val::K(v)) if self.single(d) => {
                    self.make_const(pc, d, v);
                    return true;
                }
                _ => {}
            }
        }
        match dst {
            None => {
                self.st.avail.insert(key, Val::Unit);
            }
            Some(d) if self.single(d) => {
                self.st.avail.insert(key, Val::R(d));
            }
            _ => {}
        }
        false
    }

    fn const_bytes(&self, a: Opn, len: usize) -> Option<&[u8]> {
        let Opn::A(AddrSpace::Const(c), o) = a else {
            return None;
        };
        match self.prog.consts.get(c)? {
            ConstData::Bytes(b) => b.get(o as usize..o as usize + len),
            _ => None,
        }
    }

    /// `(left, right, direction)` of dimension `dim` of a descriptor in the constant pool.
    fn const_dim(&self, dope: Opn, dim: u8) -> Option<(i64, i64, u64)> {
        let Opn::A(sp, o) = dope else { return None };
        let b = self.const_bytes(Opn::A(sp, o + 24 * u32::from(dim)), 24)?;
        let w = |k: usize| u64::from_le_bytes(b[k * 8..k * 8 + 8].try_into().unwrap_or([0; 8]));
        Some((w(0) as i64, w(1) as i64, w(2)))
    }

    fn simplify(&mut self, pc: usize) -> bool {
        let inst = self.code[pc];
        match inst {
            Inst::Move { dst, src } if self.single(dst) => match self.opn(src) {
                Opn::K(v) => {
                    self.make_const(pc, dst, v);
                    true
                }
                Opn::A(s, o) => {
                    self.make_addr(pc, dst, s, o);
                    true
                }
                Opn::R(x) => self.alias_ok(pc, dst, x),
            },
            Inst::Bin { op, dst, a, b } => {
                let (x, y) = (self.opn(a), self.opn(b));
                match simp_bin(op, x, y) {
                    Some(Simp::Fold(v)) => {
                        self.make_const(pc, dst, v);
                        true
                    }
                    Some(Simp::Same(x)) if self.alias_ok(pc, dst, x) => true,
                    _ => match (self.kopn(a), self.kopn(b)) {
                        (Some(x), Some(y)) => self.cse(pc, Some(dst), Key::Bin(op, x, y)),
                        _ => false,
                    },
                }
            }
            Inst::BinRange {
                op,
                dst,
                a,
                b,
                lo,
                hi,
                what,
            } => {
                if let (Opn::K(x), Opn::K(y), Opn::K(l), Opn::K(h)) =
                    (self.opn(a), self.opn(b), self.opn(lo), self.opn(hi))
                    && let Some(v) = fold_int(op, x, y)
                    && l <= v
                    && v <= h
                {
                    self.make_const(pc, dst, v);
                    return true;
                }
                match (self.kopn(a), self.kopn(b), self.kopn(lo), self.kopn(hi)) {
                    (Some(a), Some(b), Some(l), Some(h)) => {
                        self.cse(pc, Some(dst), Key::BinRange(op, a, b, l, h, what))
                    }
                    _ => false,
                }
            }
            Inst::Un { op, dst, a } => {
                if let Opn::K(x) = self.opn(a) {
                    let v = match op {
                        IntUnOp::Neg => x.checked_neg(),
                        IntUnOp::Abs => x.checked_abs(),
                    };
                    if let Some(v) = v {
                        self.make_const(pc, dst, v);
                        return true;
                    }
                }
                match self.kopn(a) {
                    Some(a) => self.cse(pc, Some(dst), Key::Un(op, a)),
                    None => false,
                }
            }
            Inst::Cmp { op, dst, a, b } => {
                let (x, y) = (self.opn(a), self.opn(b));
                let v = match (x, y) {
                    (Opn::K(x), Opn::K(y)) => Some(cmp(op, x.cmp(&y))),
                    (Opn::R(x), Opn::R(y)) if x == y => {
                        Some(matches!(op, CmpOp::Eq | CmpOp::Le | CmpOp::Ge))
                    }
                    _ => None,
                };
                if let Some(v) = v {
                    self.make_const(pc, dst, i64::from(v));
                    return true;
                }
                match (self.kopn(a), self.kopn(b)) {
                    (Some(x), Some(y)) => self.cse(pc, Some(dst), Key::Cmp(op, x, y)),
                    _ => false,
                }
            }
            Inst::Bool { op, dst, a, b } => {
                let (x, y) = (self.opn(a), self.opn(b));
                let v = match (op, x, y) {
                    (BoolOp::Not, Opn::K(x), _) => Some(x == 0),
                    (BoolOp::And, Opn::K(x), Opn::K(y)) => Some(x != 0 && y != 0),
                    (BoolOp::Or, Opn::K(x), Opn::K(y)) => Some(x != 0 || y != 0),
                    (BoolOp::Xor, Opn::K(x), Opn::K(y)) => Some((x != 0) != (y != 0)),
                    (BoolOp::And, Opn::K(0), _) | (BoolOp::And, _, Opn::K(0)) => Some(false),
                    (BoolOp::Or, Opn::K(k), _) | (BoolOp::Or, _, Opn::K(k)) if k != 0 => Some(true),
                    _ => None,
                };
                if let Some(v) = v {
                    self.make_const(pc, dst, i64::from(v));
                    return true;
                }
                let y = if op == BoolOp::Not {
                    Some(Opn::K(0))
                } else {
                    self.kopn(b)
                };
                match (self.kopn(a), y) {
                    (Some(x), Some(y)) => self.cse(pc, Some(dst), Key::Bool(op, x, y)),
                    _ => false,
                }
            }
            Inst::Index {
                dst,
                base,
                pos,
                stride,
            } => {
                if let (Opn::A(s, o), Opn::K(c)) = (self.opn(base), self.opn(pos))
                    && let Some(off) = i64::from(o)
                        .checked_add(c.checked_mul(i64::from(stride)).unwrap_or(-1))
                        .and_then(|v| u32::try_from(v).ok())
                    && (s != AddrSpace::Frame || off <= self.frame)
                {
                    self.make_addr(pc, dst, s, off);
                    return true;
                }
                if let (Opn::R(x), Opn::K(0)) = (self.opn(base), self.opn(pos))
                    && self.alias_ok(pc, dst, x)
                {
                    return true;
                }
                match (self.kopn(base), self.kopn(pos)) {
                    (Some(x), Some(y)) => self.cse(pc, Some(dst), Key::Index(x, y, stride)),
                    _ => false,
                }
            }
            Inst::CondBr { c, t, f } => {
                if let Opn::K(v) = self.opn(c) {
                    self.code[pc] = Inst::Br {
                        to: if v != 0 { t } else { f },
                    };
                    return true;
                }
                false
            }
            Inst::Case { v, table } => {
                if let Opn::K(x) = self.opn(v)
                    && let Some(t) = self.prog.cases.get(table)
                {
                    let to = t
                        .arms
                        .iter()
                        .find(|a| a.0 <= x && x <= a.1)
                        .map_or(t.default, |a| a.2);
                    self.code[pc] = Inst::Br { to };
                    return true;
                }
                false
            }
            Inst::Check {
                kind,
                v,
                lo,
                hi,
                what,
            } => {
                use CheckKind::*;
                let (nv, nl, nh) = (self.opn(v), self.opn(lo), self.opn(hi));
                let ok = match (kind, nv, nl, nh) {
                    (Range | Index, Opn::K(v), Opn::K(l), Opn::K(h)) => l <= v && v <= h,
                    (Length, Opn::K(v), Opn::K(l), _) => v == l,
                    (Null, Opn::K(v), _, _) => v != 0,
                    _ => false,
                };
                if ok {
                    self.code[pc] = dead();
                    return true;
                }
                // Operands the check does not read do not tell checks apart.
                let (nl, nh) = match kind {
                    Null => (Opn::K(0), Opn::K(0)),
                    Length => (nl, Opn::K(0)),
                    _ => (nl, nh),
                };
                match (self.key_ok(nv), self.key_ok(nl), self.key_ok(nh)) {
                    (true, true, true) => self.cse(pc, None, Key::Check(kind, nv, nl, nh, what)),
                    _ => false,
                }
            }
            Inst::Load { dst, addr, w } if w != Width::Sig => {
                let a = self.opn(addr);
                let n = w.bytes() as usize;
                if let Some(b) = self.const_bytes(a, n) {
                    let mut x = [0u8; 8];
                    x[..n].copy_from_slice(b);
                    self.make_const(pc, dst, u64::from_le_bytes(x) as i64);
                    return true;
                }
                match self.kopn(addr) {
                    Some(a) => self.cse(pc, Some(dst), Key::Load(a, w)),
                    None => false,
                }
            }
            Inst::DopeLen { dst, dope, dim } => {
                let d = self.opn(dope);
                if let Some((l, r, dir)) = self.const_dim(d, dim)
                    && let Some(n) = dim_len(l, r, dir == 1).filter(|&n| n <= i64::MAX as u64)
                {
                    self.make_const(pc, dst, n as i64);
                    return true;
                }
                match self.kopn(dope) {
                    Some(d) => self.cse(pc, Some(dst), Key::Dope(d, dim)),
                    None => false,
                }
            }
            Inst::Pos {
                dst,
                dope,
                dim,
                idx,
                what,
            } => {
                let (d, i) = (self.opn(dope), self.opn(idx));
                if let (Some((l, r, dir)), Opn::K(i)) = (self.const_dim(d, dim), i) {
                    let pos = match dir {
                        0 if l <= i && i <= r => Some(i.wrapping_sub(l)),
                        1 if r <= i && i <= l => Some(l.wrapping_sub(i)),
                        _ => None,
                    };
                    if let Some(p) = pos {
                        self.make_const(pc, dst, p);
                        return true;
                    }
                }
                match (self.kopn(dope), self.kopn(idx)) {
                    (Some(d), Some(i)) => self.cse(pc, Some(dst), Key::Pos(d, dim, i, what)),
                    _ => false,
                }
            }
            Inst::Field {
                dst,
                base,
                layout,
                n,
            } => match (self.kopn(base), self.kopn(layout)) {
                (Some(b), Some(l)) => self.cse(pc, Some(dst), Key::Field(b, l, n)),
                _ => false,
            },
            _ => false,
        }
    }

    fn alias_ok(&mut self, pc: usize, dst: Reg, x: Reg) -> bool {
        // `x` must be usable wherever `dst` is: defined since the last suspension in this
        // block, or the function never suspends.
        if self.nosusp || self.st.local.contains(&x) {
            self.alias(pc, dst, x)
        } else {
            false
        }
    }

    /// Remove pure definitions nobody reads.
    fn dce(&mut self) -> bool {
        let mut uses = vec![0u32; self.reg_ty.len()];
        for pc in 0..self.code.len() {
            let i = self.code[pc];
            if is_dead(&i) {
                continue;
            }
            i.visit(&mut |r, role, _| {
                if role == Role::Use
                    && let Some(u) = uses.get_mut(r.0 as usize)
                {
                    *u += 1;
                }
            });
            if let Some(rs) = i.list().and_then(|l| {
                self.operands
                    .get(l.start as usize..(l.start + l.len) as usize)
            }) {
                for r in rs {
                    if let Some(u) = uses.get_mut(r.0 as usize) {
                        *u += 1;
                    }
                }
            }
        }
        let mut any = false;
        loop {
            let mut ch = false;
            for pc in (0..self.code.len()).rev() {
                let i = self.code[pc];
                let Some(dst) = removable_dst(&i) else {
                    continue;
                };
                if uses.get(dst.0 as usize) != Some(&0) {
                    continue;
                }
                i.visit(&mut |r, role, _| {
                    if role == Role::Use
                        && let Some(u) = uses.get_mut(r.0 as usize)
                    {
                        *u = u.saturating_sub(1);
                    }
                });
                self.code[pc] = dead();
                ch = true;
            }
            any |= ch;
            if !ch {
                break;
            }
        }
        any
    }

    /// The compacted code, locations, block ranges and operands.
    #[allow(clippy::type_complexity)]
    fn finish(self, loc: &[StmtId]) -> (Vec<Inst>, Vec<StmtId>, Vec<(u32, u32)>, Vec<Reg>) {
        let mut code = Vec::with_capacity(self.code.len());
        let mut locs = Vec::with_capacity(self.code.len());
        let mut blocks = Vec::with_capacity(self.blocks.len());
        for &(s, e) in &self.blocks {
            let start = code.len() as u32;
            for (i, l) in self.code[s as usize..e as usize]
                .iter()
                .zip(&loc[s as usize..e as usize])
            {
                if !is_dead(i) {
                    code.push(*i);
                    locs.push(*l);
                }
            }
            blocks.push((start, code.len() as u32));
        }
        (code, locs, blocks, self.operands)
    }
}

/// The register an instruction defines, if it can be dropped when that is never read: no
/// side effect and no failure other than a broken invariant.
fn removable_dst(i: &Inst) -> Option<Reg> {
    match *i {
        Inst::Const { dst, .. }
        | Inst::ConstF { dst, .. }
        | Inst::Move { dst, .. }
        | Inst::AddrOf { dst, .. }
        | Inst::Index { dst, .. }
        | Inst::Cmp { dst, .. }
        | Inst::FCmp { dst, .. }
        | Inst::Bool { dst, .. }
        | Inst::IToF { dst, .. } => Some(dst),
        _ => None,
    }
}

fn cmp(op: CmpOp, o: std::cmp::Ordering) -> bool {
    use std::cmp::Ordering::*;
    match op {
        CmpOp::Eq => o == Equal,
        CmpOp::Ne => o != Equal,
        CmpOp::Lt => o == Less,
        CmpOp::Le => o != Greater,
        CmpOp::Gt => o == Greater,
        CmpOp::Ge => o != Less,
    }
}

/// The checked result of `x op y` when it succeeds (the interpreter's `int_op`).
fn fold_int(op: IntOp, x: i64, y: i64) -> Option<i64> {
    match op {
        IntOp::Add => x.checked_add(y),
        IntOp::Sub => x.checked_sub(y),
        IntOp::Mul => x.checked_mul(y),
        IntOp::Div if y != 0 => x.checked_div(y),
        IntOp::Rem if y != 0 => Some(x.checked_rem(y).unwrap_or(0)),
        IntOp::Mod if y != 0 => {
            let r = x.checked_rem(y).unwrap_or(0);
            Some(if r != 0 && ((r < 0) != (y < 0)) {
                r + y
            } else {
                r
            })
        }
        _ => None,
    }
}

fn simp_bin(op: IntOp, x: Opn, y: Opn) -> Option<Simp> {
    use Opn::*;
    match (op, x, y) {
        (_, K(a), K(b)) => fold_int(op, a, b).map(Simp::Fold),
        (IntOp::Add, R(r), K(0)) | (IntOp::Add, K(0), R(r)) => Some(Simp::Same(r)),
        (IntOp::Sub, R(r), K(0)) => Some(Simp::Same(r)),
        (IntOp::Sub, R(a), R(b)) if a == b => Some(Simp::Fold(0)),
        (IntOp::Mul, R(r), K(1)) | (IntOp::Mul, K(1), R(r)) => Some(Simp::Same(r)),
        (IntOp::Mul, K(0), R(_)) | (IntOp::Mul, R(_), K(0)) => Some(Simp::Fold(0)),
        (IntOp::Div, R(r), K(1)) => Some(Simp::Same(r)),
        (IntOp::Rem | IntOp::Mod, R(_), K(1)) => Some(Simp::Fold(0)),
        _ => None,
    }
}
