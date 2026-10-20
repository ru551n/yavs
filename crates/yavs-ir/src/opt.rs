//! Machine-independent optimisation of a function's code, run once per function when the
//! program is finished (after flag propagation, before resume points are numbered and the
//! verifier runs).
//!
//! Lowering emits every operand where it is used, so the code is full of constants, repeated
//! address and descriptor computations and loads of what was just stored. This pass walks
//! the extended basic blocks of a function (a block and, depth first, the blocks only it can
//! reach), keeping a table of available values, and
//!
//! - folds constants and trivial arithmetic (`x * 1`, `x + 0`, `x * 0`) and branches on
//!   constants,
//! - turns an offset from a known address into a known address (`AddrOf`),
//! - reads constant-pool descriptors and scalars at compile time,
//! - removes an instruction that repeats an earlier one (same operands, same result),
//!   including checks that cannot fail a second time,
//! - forwards a stored value to a later load of the same address,
//! - drops pure definitions nobody reads.
//!
//! Invariants it keeps (ir.md 1.2): no register becomes live across a suspension (a
//! register is replaced by another only when the other is defined since the last
//! suspension on the path, or the function never suspends; every table is dropped at a
//! suspension), every surviving instruction keeps its statement, `Stmt` and `Cover` are
//! never touched, a check is removed only when it provably passes, and stores are never
//! removed (debuggers read frame slots). Memory facts are dropped at calls, at writes
//! through pointers and, for what a debugger may write, at every `Stmt`. Functions of kind
//! `Elab` and `Thunk` run once per instance and are left alone. `YAVS_NO_IROPT=1` turns the
//! pass off (differential testing).

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::OnceLock;

use crate::{
    AddrSpace, BlockId, BoolOp, CheckKind, CheckMsgId, CmpOp, ConstData, FuncFlags, FuncId,
    FuncKind, Inst, IntOp, IntUnOp, Program, Reg, RegTy, Role, StmtId, Ty, TyId, Width, dim_len,
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

/// A fast, non-cryptographic hasher: the tables are small, built per function and never
/// see hostile keys.
#[derive(Default)]
struct Fx(u64);

impl Fx {
    #[inline]
    fn add(&mut self, w: u64) {
        self.0 = (self.0.rotate_left(5) ^ w).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
}

impl Hasher for Fx {
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.add(u64::from(*b));
        }
    }
    fn write_u8(&mut self, i: u8) {
        self.add(u64::from(i));
    }
    fn write_u32(&mut self, i: u32) {
        self.add(u64::from(i));
    }
    fn write_u64(&mut self, i: u64) {
        self.add(i);
    }
    fn write_i64(&mut self, i: i64) {
        self.add(i as u64);
    }
    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }
    fn write_isize(&mut self, i: isize) {
        self.add(i as u64);
    }
    fn finish(&self) -> u64 {
        self.0
    }
}

type FxMap<K, V> = HashMap<K, V, BuildHasherDefault<Fx>>;

/// The optimised copy of function `id`, or `None` when the pass is off or leaves it alone.
/// Reads only what lowering produced of other functions (flags, kinds, types), so the result
/// does not depend on which function was optimised first.
pub(crate) fn optimize_func(prog: &Program, id: FuncId) -> Option<Box<crate::Func>> {
    if !enabled() {
        return None;
    }
    let raw = prog.funcs.get(id)?;
    if matches!(raw.kind, FuncKind::Elab | FuncKind::Thunk) || raw.code.len() < 2 {
        return None;
    }
    let mut layouts = FxMap::default();
    let mut o = Opt::new(
        prog,
        raw,
        raw.code.clone(),
        raw.operands.clone(),
        raw.nested,
        &mut layouts,
    );
    o.run();
    let mut loc = raw.loc.clone();
    let blocks = o.changed.then(|| o.compact(&mut loc));
    let mut f = raw.shell();
    if let Some(blocks) = blocks {
        for (i, nb) in blocks.iter().enumerate() {
            if let Some(b) = f.blocks.get_mut(BlockId(i as u32)) {
                b.start = nb.0;
                b.end = nb.1;
            }
        }
    }
    f.code = o.code;
    f.loc = loc;
    f.operands = o.operands;
    f.reg_ty.extend(o.new_regs);
    crate::build::number_func(&mut f, |c| {
        prog.funcs
            .get(c)
            .is_some_and(|c| c.flags.contains(FuncFlags::MAY_SUSPEND))
    });
    #[cfg(debug_assertions)]
    {
        let mut errs = Vec::new();
        crate::verify::verify_func(prog, id, &f, &mut errs);
        assert!(errs.is_empty(), "optimiser broke {}: {errs:?}", f.name);
    }
    Some(Box::new(f))
}

/// The registers `i` defines: the common instructions directly, the rest through
/// [`Inst::visit`] (the analysis runs over every instruction, so this one is worth a fast
/// path; debug builds check the two agree).
fn for_defs(i: &Inst, f: impl FnMut(Reg)) {
    #[cfg(debug_assertions)]
    {
        let (mut fast, mut slow) = (Vec::new(), Vec::new());
        for_defs_fast(i, |r| fast.push(r));
        i.visit(&mut |r, role, _| {
            if role == Role::Def {
                slow.push(r);
            }
        });
        assert_eq!(fast, slow, "definitions of {i:?}");
    }
    for_defs_fast(i, f);
}

fn for_defs_fast(i: &Inst, mut f: impl FnMut(Reg)) {
    match *i {
        Inst::Const { dst, .. }
        | Inst::ConstF { dst, .. }
        | Inst::Move { dst, .. }
        | Inst::AddrOf { dst, .. }
        | Inst::Load { dst, .. }
        | Inst::Index { dst, .. }
        | Inst::Bin { dst, .. }
        | Inst::BinRange { dst, .. }
        | Inst::Cmp { dst, .. }
        | Inst::Bool { dst, .. }
        | Inst::Pos { dst, .. }
        | Inst::DopeLen { dst, .. }
        | Inst::Field { dst, .. }
        | Inst::Un { dst, .. } => f(dst),
        Inst::Store { .. }
        | Inst::Stmt
        | Inst::Br { .. }
        | Inst::CondBr { .. }
        | Inst::Ret { .. }
        | Inst::Check { .. }
        | Inst::Cover { .. }
        | Inst::Copy { .. } => {}
        _ => i.visit(&mut |r, role, _| {
            if role == Role::Def {
                f(r);
            }
        }),
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

/// Which memory an address points into, where known.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Cls {
    Space(AddrSpace),
    Heap,
    Temp,
}

/// Is the byte range `[a, a + n)` clear of every range in `rs`?
fn clear_of(rs: &[(i64, i64)], a: i64, n: i64) -> bool {
    rs.iter()
        .all(|&(s, z)| a + n <= s || s.saturating_add(z) <= a)
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
    /// The temporary layout of a type under a descriptor: gone at the next `Stmt`.
    Layout(TyId, Opn),
}

impl Key {
    /// The address operand and the byte range read from it, for memory-dependent keys.
    fn region(&self) -> Option<(Opn, i64, i64)> {
        match *self {
            Key::Load(a, w) => Some((a, 0, i64::from(w.bytes()))),
            Key::Dope(a, d) | Key::Pos(a, d, ..) => Some((a, i64::from(d) * 24, 24)),
            Key::Field(_, l, n) => Some((l, 16 + 8 * i64::from(n), 8)),
            Key::Layout(_, d) => Some((d, 0, 24 * 8)),
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

/// Most memory facts kept at once.
const MEM_CAP: usize = 64;

/// The available expressions on the current path, with an undo log so that siblings of an
/// extended basic block do not see each other's facts.
#[derive(Default)]
struct Table {
    pure: FxMap<Key, Val>,
    mem: FxMap<Key, Val>,
    log: Vec<(Key, Option<Val>)>,
}

impl Table {
    fn put(&mut self, k: Key, v: Val) {
        let old = if k.region().is_some() {
            // ponytail: a full table stops learning memory facts; every store and statement
            // scans it, so it stays small. An index by address if this loses much.
            if self.mem.len() >= MEM_CAP {
                return;
            }
            self.mem.insert(k.clone(), v)
        } else {
            self.pure.insert(k.clone(), v)
        };
        self.log.push((k, old));
    }

    fn get(&self, k: &Key) -> Option<Val> {
        if k.region().is_some() {
            self.mem.get(k).copied()
        } else {
            self.pure.get(k).copied()
        }
    }

    /// Forget the memory-dependent entries `drop` selects.
    fn forget(&mut self, mut drop: impl FnMut(&Key) -> bool) {
        let gone: Vec<Key> = self.mem.keys().filter(|k| drop(k)).cloned().collect();
        for k in gone {
            if let Some(v) = self.mem.remove(&k) {
                self.log.push((k, Some(v)));
            }
        }
    }

    fn clear(&mut self) {
        self.forget(|_| true);
        let gone: Vec<Key> = self.pure.keys().cloned().collect();
        for k in gone {
            if let Some(v) = self.pure.remove(&k) {
                self.log.push((k, Some(v)));
            }
        }
    }

    fn rollback(&mut self, mark: usize) {
        while self.log.len() > mark {
            let Some((k, old)) = self.log.pop() else {
                break;
            };
            let m = if k.region().is_some() {
                &mut self.mem
            } else {
                &mut self.pure
            };
            match old {
                Some(v) => {
                    m.insert(k, v);
                }
                None => {
                    m.remove(&k);
                }
            }
        }
    }
}

/// What a simplified arithmetic instruction becomes.
enum Simp {
    Fold(i64),
    Same(Reg),
}

/// Where the depth-first walk of the extended basic blocks goes next.
enum Step {
    Enter(usize),
    Leave {
        block: usize,
        log: usize,
        susp: (u32, u32),
    },
}

struct Opt<'a> {
    prog: &'a Program,
    f: &'a crate::Func,
    code: Vec<Inst>,
    operands: Vec<Reg>,
    blocks: Vec<(u32, u32)>,
    /// Nested subprograms exist: they reach this frame through static links.
    nested: bool,
    /// Frame ranges a debugger may write: scalar locals and parameters.
    named: Vec<(i64, i64)>,
    /// Frame ranges whose address is used other than directly by a load or store (or the
    /// whole frame when nested subprograms can reach it); computed when first needed.
    esc: Option<Vec<(i64, i64)>>,
    /// Definitions per register (saturating).
    defs: Vec<u8>,
    /// Registers defined once, by a `Const`.
    kc: Vec<Option<i64>>,
    /// Registers defined once, by an `AddrOf`.
    ka: Vec<Option<(AddrSpace, u32)>>,
    /// Where single-definition address registers point.
    cls: Vec<Option<Cls>>,
    /// Registers replaced by another (both defined once): `rep[r] == r` if not.
    rep: Vec<u32>,
    nrep: usize,
    /// The function has no suspension point.
    nosusp: bool,
    /// The block being walked.
    cur: usize,
    tbl: Table,
    /// Where each register is (last) defined: block and instruction.
    def_blk: Vec<u32>,
    def_pc: Vec<u32>,
    /// Depth of each block on the current path of the walk (`u32::MAX`: not on it) and the
    /// depth of the walk.
    path_depth: Vec<u32>,
    depth: u32,
    /// Block depth and instruction of the last suspension on the path: what was defined
    /// before it is not usable.
    susp_depth: u32,
    susp_pc: u32,
    /// Registers added by the pass (after the function's own) and instructions to insert
    /// before an instruction: `(pc, instruction)`.
    new_regs: Vec<RegTy>,
    inserts: Vec<(usize, Inst)>,
    /// Result registers of `LayoutOf` of a type whose layout is the same under every
    /// descriptor: the type.
    slay: FxMap<Reg, TyId>,
    /// Field offsets of the static record layouts met so far (shared by the functions of one
    /// batch).
    layouts: &'a mut FxMap<TyId, Option<Vec<u64>>>,
    /// Readers per register, as left by the dead-code sweep.
    uses: Vec<u32>,
    changed: bool,
}

impl<'a> Opt<'a> {
    fn new(
        prog: &'a Program,
        f: &'a crate::Func,
        code: Vec<Inst>,
        operands: Vec<Reg>,
        nested: bool,
        layouts: &'a mut FxMap<TyId, Option<Vec<u64>>>,
    ) -> Self {
        let n = f.reg_ty.len();
        Opt {
            prog,
            f,
            code,
            operands,
            blocks: f.blocks.values().iter().map(|b| (b.start, b.end)).collect(),
            nested,
            // A debugger writes a scalar local in its slot; a composite through the pointer
            // its slot holds (`write_slot`), so only scalar slots are variables here.
            named: f
                .locals
                .iter()
                .filter(|l| {
                    prog.types
                        .get(l.ty)
                        .is_some_and(|t| t.scalar_size().is_some())
                })
                .map(|l| (i64::from(l.slot), i64::from(l.size)))
                .collect(),
            esc: None,
            defs: vec![0; n],
            kc: vec![None; n],
            ka: vec![None; n],
            cls: vec![None; n],
            rep: (0..n as u32).collect(),
            nrep: 0,
            nosusp: f.body.is_none(),
            cur: 0,
            tbl: Table::default(),
            def_blk: vec![0; n],
            def_pc: vec![0; n],
            path_depth: vec![u32::MAX; f.blocks.len()],
            depth: 0,
            susp_depth: 0,
            susp_pc: 0,
            new_regs: Vec::new(),
            inserts: Vec::new(),
            slay: FxMap::default(),
            layouts,
            uses: Vec::new(),
            changed: false,
        }
    }

    fn is_susp(&self, i: &Inst) -> bool {
        match *i {
            Inst::Suspend { .. } => true,
            Inst::Call { func, .. } => self
                .prog
                .funcs
                .get(func)
                .is_some_and(|f| f.flags.contains(FuncFlags::MAY_SUSPEND)),
            _ => false,
        }
    }

    /// A register defined exactly once, holding a plain value.
    fn single(&self, r: Reg) -> bool {
        self.defs.get(r.0 as usize) == Some(&1)
            && matches!(self.rty(r), Some(RegTy::I64 | RegTy::Ptr))
    }

    fn rty(&self, r: Reg) -> Option<RegTy> {
        let n0 = self.f.reg_ty.len();
        let i = r.0 as usize;
        if i < n0 {
            self.f.reg_ty.get(i).copied()
        } else {
            self.new_regs.get(i - n0).copied()
        }
    }

    /// A new register holding the constant `v`, defined by an instruction inserted before
    /// `pc` (in block `b`).
    fn const_reg(&mut self, b: usize, pc: usize, v: i64) -> Reg {
        let r = Reg(self.defs.len() as u32);
        self.new_regs.push(RegTy::I64);
        self.defs.push(1);
        self.kc.push(Some(v));
        self.ka.push(None);
        self.cls.push(None);
        self.rep.push(r.0);
        self.def_blk.push(b as u32);
        self.def_pc.push(pc as u32);
        self.inserts.push((pc, Inst::Const { dst: r, v }));
        r
    }

    fn resolve(&self, mut r: Reg) -> Reg {
        while let Some(&x) = self.rep.get(r.0 as usize) {
            if x == r.0 {
                break;
            }
            r = Reg(x);
        }
        r
    }

    fn opn(&self, r: Reg) -> Opn {
        let r = self.resolve(r);
        let i = r.0 as usize;
        if let Some(Some(v)) = self.kc.get(i) {
            Opn::K(*v)
        } else if let Some(Some((s, o))) = self.ka.get(i) {
            Opn::A(*s, *o)
        } else {
            Opn::R(r)
        }
    }

    fn cls_of(&self, o: Opn) -> Option<Cls> {
        match o {
            Opn::A(s, _) => Some(Cls::Space(s)),
            Opn::R(r) => self.cls.get(r.0 as usize).copied().flatten(),
            Opn::K(_) => None,
        }
    }

    fn set_cls(&mut self, r: Reg, c: Cls) {
        if let Some(x) = self.cls.get_mut(r.0 as usize) {
            *x = Some(c);
        }
    }

    /// Definition counts, constants, addresses, and the offsets of known addresses folded
    /// (`Index` of a known address by a known position).
    fn analyse(&mut self) {
        for b in 0..self.blocks.len() {
            for pc in self.range(b) {
                let i = &self.code[pc];
                if matches!(i, Inst::Suspend { .. }) {
                    self.nosusp = false;
                }
                if let Inst::Call { func, .. } = *i
                    && self
                        .prog
                        .funcs
                        .get(func)
                        .is_some_and(|f| f.flags.contains(FuncFlags::MAY_SUSPEND))
                {
                    self.nosusp = false;
                }
                let (defs, blk, at) = (&mut self.defs, &mut self.def_blk, &mut self.def_pc);
                for_defs(i, |r| {
                    if let Some(d) = defs.get_mut(r.0 as usize) {
                        *d = d.saturating_add(1);
                        blk[r.0 as usize] = b as u32;
                        at[r.0 as usize] = pc as u32;
                    }
                });
            }
        }
        for pc in 0..self.code.len() {
            match self.code[pc] {
                Inst::Const { dst, v } if self.single(dst) => {
                    self.kc[dst.0 as usize] = Some(v);
                }
                Inst::AddrOf { dst, space, off } if self.single(dst) => {
                    self.ka[dst.0 as usize] = Some((space, off));
                    self.set_cls(dst, Cls::Space(space));
                }
                Inst::Deref { dst, .. } if self.single(dst) => self.set_cls(dst, Cls::Heap),
                Inst::LayoutOf { dst, ty, .. }
                    if self.single(dst) && Ty::desc_size(&self.prog.types, ty) == Some(0) =>
                {
                    self.set_cls(dst, Cls::Temp);
                    self.slay.insert(dst, ty);
                }
                Inst::Alloc { dst, .. } | Inst::LayoutOf { dst, .. } if self.single(dst) => {
                    self.set_cls(dst, Cls::Temp);
                }
                Inst::Index { dst, base, pos, .. } if self.single(dst) => {
                    if self.fold_index(pc) {
                        continue;
                    }
                    if let Some(c) = self.cls_of(Opn::R(base)) {
                        self.set_cls(dst, c);
                    }
                    let _ = pos;
                }
                Inst::Field { dst, base, .. } if self.single(dst) => {
                    if let Some(c) = self.cls_of(Opn::R(base)) {
                        self.set_cls(dst, c);
                    }
                }
                _ => {}
            }
        }
    }

    /// `Index` of a known address by a known position becomes an `AddrOf`.
    fn fold_index(&mut self, pc: usize) -> bool {
        let Inst::Index {
            dst,
            base,
            pos,
            stride,
        } = self.code[pc]
        else {
            return false;
        };
        if let (Opn::A(s, o), Opn::K(c)) = (self.opn(base), self.opn(pos))
            && let Some(off) = i64::from(o)
                .checked_add(c.checked_mul(i64::from(stride)).unwrap_or(-1))
                .and_then(|v| u32::try_from(v).ok())
            && (s != AddrSpace::Frame || off <= self.f.frame)
        {
            self.make_addr(pc, dst, s, off);
            return true;
        }
        false
    }

    /// The `k`-th successor of block `b`.
    fn succ(&self, b: usize, k: usize) -> Option<usize> {
        let (s, e) = self.blocks[b];
        if e <= s {
            return None;
        }
        let t = match self.code[e as usize - 1] {
            Inst::Br { to } => (k == 0).then_some(to),
            Inst::CondBr { t, f, .. } => match k {
                0 => Some(t),
                1 => Some(f),
                _ => None,
            },
            Inst::Case { table, .. } => {
                let t = self.prog.cases.get(table)?;
                if k == 0 {
                    Some(t.default)
                } else {
                    t.arms.get(k - 1).map(|a| a.2)
                }
            }
            _ => None,
        }?;
        ((t.0 as usize) < self.blocks.len()).then_some(t.0 as usize)
    }

    fn run(&mut self) {
        self.analyse();
        let n = self.blocks.len();
        // Block structure: predecessor counts, the reverse post-order for the walk, and
        // the blocks only one block reaches (as lists through `child`).
        const NONE: u32 = u32::MAX;
        let mut preds = vec![0u32; n];
        let mut first = vec![NONE; n];
        for b in 0..n {
            let mut k = 0;
            while let Some(t) = self.succ(b, k) {
                preds[t] += 1;
                first[t] = b as u32;
                k += 1;
            }
        }
        let mut head = vec![NONE; n];
        let mut child = vec![NONE; n];
        let mut is_root = vec![false; n];
        for b in (0..n).rev() {
            is_root[b] = b == 0
                || preds[b] != 1
                || first[b] == b as u32
                || self.f.body.is_some_and(|x| x.0 as usize == b);
            if !is_root[b] {
                let p = first[b] as usize;
                child[b] = head[p];
                head[p] = b as u32;
            }
        }
        let mut order = Vec::with_capacity(n);
        let mut seen = vec![false; n];
        // Post-order by an explicit stack of (block, next successor).
        let mut stack = vec![(0usize, 0usize)];
        seen[0] = true;
        while let Some(&(b, k)) = stack.last() {
            if let Some(s) = self.succ(b, k) {
                if let Some(top) = stack.last_mut() {
                    top.1 += 1;
                }
                if !seen[s] {
                    seen[s] = true;
                    stack.push((s, 0));
                }
            } else {
                order.push(b);
                stack.pop();
            }
        }
        order.reverse();
        let mut visited = vec![false; n];
        let mut work: Vec<Step> = Vec::new();
        // The declarative part of a process runs once: its blocks come before the body.
        let cold = self.f.body.map_or(0, |b| b.0 as usize);
        for r in order.iter().copied().chain(0..n) {
            if !is_root[r] || visited[r] || r < cold {
                continue;
            }
            self.susp_depth = 0;
            self.susp_pc = 0;
            work.push(Step::Enter(r));
            while let Some(step) = work.pop() {
                match step {
                    Step::Enter(b) => {
                        visited[b] = true;
                        work.push(Step::Leave {
                            block: b,
                            log: self.tbl.log.len(),
                            susp: (self.susp_depth, self.susp_pc),
                        });
                        self.depth += 1;
                        self.path_depth[b] = self.depth;
                        self.walk(b);
                        let mut c = head[b];
                        while c != NONE {
                            work.push(Step::Enter(c as usize));
                            c = child[c as usize];
                        }
                    }
                    Step::Leave { block, log, susp } => {
                        self.tbl.rollback(log);
                        self.path_depth[block] = u32::MAX;
                        self.depth -= 1;
                        (self.susp_depth, self.susp_pc) = susp;
                    }
                }
            }
        }
        self.dce();
    }

    fn range(&self, b: usize) -> std::ops::Range<usize> {
        self.blocks[b].0 as usize..self.blocks[b].1 as usize
    }

    /// The constant `dst` becomes.
    fn make_const(&mut self, pc: usize, dst: Reg, v: i64) {
        self.code[pc] = Inst::Const { dst, v };
        if self.single(dst) {
            self.kc[dst.0 as usize] = Some(v);
        }
        self.changed = true;
    }

    fn make_addr(&mut self, pc: usize, dst: Reg, space: AddrSpace, off: u32) {
        self.code[pc] = Inst::AddrOf { dst, space, off };
        if self.single(dst) {
            self.ka[dst.0 as usize] = Some((space, off));
            self.set_cls(dst, Cls::Space(space));
        }
        self.changed = true;
    }

    /// Drop `pc`, using `x` wherever `dst` was read. Only when both are defined once.
    fn alias(&mut self, pc: usize, dst: Reg, x: Reg) -> bool {
        if self.single(dst) && self.single(x) && dst != x {
            self.rep[dst.0 as usize] = x.0;
            self.nrep += 1;
            self.code[pc] = dead();
            self.changed = true;
            true
        } else {
            false
        }
    }

    /// [`Opt::alias`] when `x` is usable where `dst` is: defined on this path since the
    /// last suspension, or the function never suspends.
    fn alias_ok(&mut self, pc: usize, dst: Reg, x: Reg) -> bool {
        let local = self.nosusp || {
            let pd = self.path_depth[self.def_blk[x.0 as usize] as usize];
            pd != u32::MAX
                && (pd > self.susp_depth
                    || (pd == self.susp_depth && self.def_pc[x.0 as usize] > self.susp_pc))
        };
        local && self.alias(pc, dst, x)
    }

    fn walk(&mut self, b: usize) {
        self.cur = b;
        for pc in self.range(b) {
            match self.code[pc] {
                // Known constants and addresses are tracked by value; nothing to do.
                Inst::Const { dst, .. } | Inst::ConstF { dst, .. } | Inst::AddrOf { dst, .. }
                    if self.single(dst) =>
                {
                    continue;
                }
                Inst::Unreachable { what } if what == DEAD_MSG => continue,
                _ => {}
            }
            self.simplify(pc);
            let i = self.code[pc];
            if is_dead(&i) {
                continue;
            }
            if self.is_susp(&i) {
                self.tbl.clear();
                self.susp_depth = self.depth;
                self.susp_pc = pc as u32;
            } else {
                self.effect(&i);
            }
        }
    }

    fn ensure_esc(&mut self) {
        if self.esc.is_none() {
            self.esc = Some(self.escapes());
        }
    }

    /// Frame ranges whose address leaves direct load/store use.
    fn escapes(&self) -> Vec<(i64, i64)> {
        if self.nested {
            return vec![(0, i64::MAX)];
        }
        let frame = i64::from(self.f.frame);
        // The object at `o` ends at the next named local, or at the end of the frame; the
        // descriptor of an array variable `[data, dope, handle, descriptor]` is its last
        // `24 * dims` bytes.
        let objs = |o: i64| {
            self.f
                .locals
                .iter()
                .map(|l| (i64::from(l.slot), i64::from(l.size), l.ty))
                .chain(self.f.params.iter().map(|p| {
                    (
                        i64::from(p.slot),
                        i64::from(p.width.bytes()),
                        TyId(u32::MAX),
                    )
                }))
                .filter(move |&(s, z, _)| s <= o && o < s + z)
        };
        let range = |o: i64, n: Option<i64>| match n {
            Some(n) => (o, n),
            None => match objs(o).next() {
                Some((s, z, t)) => match self.prog.types.get(t) {
                    Some(Ty::Array { dims, .. }) if o == s + 24 => (o, 24 * i64::from(*dims)),
                    _ => (s, z),
                },
                None => {
                    let end = self
                        .f
                        .locals
                        .iter()
                        .map(|l| i64::from(l.slot))
                        .chain(self.f.params.iter().map(|p| i64::from(p.slot)))
                        .filter(|&s| s > o)
                        .min()
                        .unwrap_or(frame);
                    (o, (end - o).max(1))
                }
            },
        };
        let mut esc = Vec::new();
        let mut mark = |r: Reg, n: Option<i64>| {
            if let Some(Some((AddrSpace::Frame, o))) = self.ka.get(self.resolve(r).0 as usize) {
                esc.push(range(i64::from(*o), n));
            }
        };
        for i in &self.code {
            match *i {
                Inst::Load { .. } => {}
                Inst::Store { src, .. } => mark(src, None),
                Inst::Copy { dst, src, bytes } => {
                    let n = match self.opn(bytes) {
                        Opn::K(n) => Some(n),
                        _ => None,
                    };
                    mark(dst, n);
                    mark(src, n);
                }
                _ => {
                    i.visit(&mut |r, role, _| {
                        if role == Role::Use {
                            mark(r, None);
                        }
                    });
                    if let Some(l) = i.list() {
                        for k in l.start as usize..(l.start + l.len) as usize {
                            if let Some(&r) = self.operands.get(k) {
                                mark(r, None);
                            }
                        }
                    }
                }
            }
        }
        esc
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
                        self.tbl.put(Key::Load(a, Width::B8), v);
                    }
                }
            }
            // A debugger stopped here may have written a variable: forget what is known of
            // scalar locals and of memory reached through pointers; the rest stays.
            Inst::Stmt => {
                let named = &self.named;
                let this: &Opt = self;
                let drop_key = |k: &Key| match k.region() {
                    _ if matches!(k, Key::Layout(..)) => true,
                    None => false,
                    Some((Opn::A(AddrSpace::Const(_), _), ..)) => false,
                    Some((Opn::A(AddrSpace::Frame, o), lo, n)) => {
                        !clear_of(named, i64::from(o) + lo, n)
                    }
                    Some((a, ..)) => this.cls_of(a) != Some(Cls::Temp),
                };
                let gone: Vec<Key> = self
                    .tbl
                    .mem
                    .keys()
                    .filter(|k| drop_key(k))
                    .cloned()
                    .collect();
                for k in gone {
                    if let Some(v) = self.tbl.mem.remove(&k) {
                        self.tbl.log.push((k, Some(v)));
                    }
                }
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
            | Inst::LayoutOf { .. }
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
            // Writes through a pointer, calls: anything may have changed.
            _ => self.kill_all(),
        }
    }

    /// A call or an unknown write: nothing survives but constants and frame slots that
    /// neither a debugger shows nor any pointer reaches.
    fn kill_all(&mut self) {
        if self.tbl.mem.is_empty() {
            return;
        }
        self.ensure_esc();
        let esc = self.esc.as_deref().unwrap_or(&[]);
        let named = &self.named;
        self.tbl.forget(|k| match k.region() {
            None => false,
            Some((Opn::A(AddrSpace::Const(_), _), ..)) => false,
            Some((Opn::A(AddrSpace::Frame, o), lo, n)) => {
                let a = i64::from(o) + lo;
                !(clear_of(named, a, n) && clear_of(esc, a, n))
            }
            Some(_) => true,
        });
    }

    /// A store of `len` bytes at `at`.
    fn kill(&mut self, at: Opn, len: i64) {
        if self.tbl.mem.is_empty() {
            return;
        }
        self.ensure_esc();
        let this: &Opt = self;
        let esc = this.esc.as_deref().unwrap_or(&[]);
        let cs = this.cls_of(at);
        let keep = |k: &Key| {
            let Some((a2, lo, n)) = k.region() else {
                return true;
            };
            let ce = this.cls_of(a2);
            if matches!(ce, Some(Cls::Space(AddrSpace::Const(_)))) {
                return true;
            }
            let apart = matches!((cs, ce), (Some(x), Some(y)) if x != y);
            match (at, a2) {
                (Opn::A(s1, o1), Opn::A(s2, o2)) => {
                    s1 != s2 || {
                        let (a, b) = (i64::from(o1), i64::from(o2) + lo);
                        a + len <= b || b + n <= a
                    }
                }
                // A pointer reaches a frame slot only if the slot's address escaped.
                (_, Opn::A(AddrSpace::Frame, o2)) => apart || clear_of(esc, i64::from(o2) + lo, n),
                (Opn::A(AddrSpace::Frame, o1), _) => apart || clear_of(esc, i64::from(o1), len),
                _ => apart,
            }
        };
        let gone: Vec<Key> = self.tbl.mem.keys().filter(|k| !keep(k)).cloned().collect();
        for k in gone {
            if let Some(v) = self.tbl.mem.remove(&k) {
                self.tbl.log.push((k, Some(v)));
            }
        }
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
    fn cse(&mut self, pc: usize, dst: Option<Reg>, key: Key) {
        if let Some(v) = self.tbl.get(&key) {
            match (dst, v) {
                (None, Val::Unit) => {
                    self.code[pc] = dead();
                    self.changed = true;
                    return;
                }
                (Some(d), Val::R(x)) if self.alias(pc, d, x) => return,
                (Some(d), Val::K(v)) if self.single(d) => {
                    self.make_const(pc, d, v);
                    return;
                }
                _ => {}
            }
        }
        match dst {
            None => self.tbl.put(key, Val::Unit),
            Some(d) if self.single(d) => self.tbl.put(key, Val::R(d)),
            _ => {}
        }
    }

    /// Offset of field `n` in record type `ty`, which has the same layout under every
    /// descriptor.
    fn static_offset(&mut self, ty: TyId, n: u32) -> Option<u64> {
        let (types, consts) = (&self.prog.types, &self.prog.consts);
        self.layouts
            .entry(ty)
            .or_insert_with(|| Ty::layout(types, consts, ty, &[]).ok().map(|l| l.0.offsets))
            .as_ref()?
            .get(n as usize)
            .copied()
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

    fn simplify(&mut self, pc: usize) {
        let inst = self.code[pc];
        match inst {
            Inst::Move { dst, src } if self.single(dst) => match self.opn(src) {
                Opn::K(v) => self.make_const(pc, dst, v),
                Opn::A(s, o) => self.make_addr(pc, dst, s, o),
                Opn::R(x) => {
                    self.alias_ok(pc, dst, x);
                }
            },
            Inst::Bin { op, dst, a, b } => {
                match simp_bin(op, self.opn(a), self.opn(b)) {
                    Some(Simp::Fold(v)) => return self.make_const(pc, dst, v),
                    Some(Simp::Same(x)) if self.alias_ok(pc, dst, x) => return,
                    _ => {}
                }
                if let (Some(x), Some(y)) = (self.kopn(a), self.kopn(b)) {
                    self.cse(pc, Some(dst), Key::Bin(op, x, y));
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
                    return self.make_const(pc, dst, v);
                }
                if let (Some(a), Some(b), Some(l), Some(h)) =
                    (self.kopn(a), self.kopn(b), self.kopn(lo), self.kopn(hi))
                {
                    self.cse(pc, Some(dst), Key::BinRange(op, a, b, l, h, what));
                }
            }
            Inst::Un { op, dst, a } => {
                if let Opn::K(x) = self.opn(a) {
                    let v = match op {
                        IntUnOp::Neg => x.checked_neg(),
                        IntUnOp::Abs => x.checked_abs(),
                    };
                    if let Some(v) = v {
                        return self.make_const(pc, dst, v);
                    }
                }
                if let Some(a) = self.kopn(a) {
                    self.cse(pc, Some(dst), Key::Un(op, a));
                }
            }
            Inst::Cmp { op, dst, a, b } => {
                let v = match (self.opn(a), self.opn(b)) {
                    (Opn::K(x), Opn::K(y)) => Some(cmp(op, x.cmp(&y))),
                    (Opn::R(x), Opn::R(y)) if x == y => {
                        Some(matches!(op, CmpOp::Eq | CmpOp::Le | CmpOp::Ge))
                    }
                    _ => None,
                };
                if let Some(v) = v {
                    return self.make_const(pc, dst, i64::from(v));
                }
                if let (Some(x), Some(y)) = (self.kopn(a), self.kopn(b)) {
                    self.cse(pc, Some(dst), Key::Cmp(op, x, y));
                }
            }
            Inst::Bool { op, dst, a, b } => {
                let v = match (op, self.opn(a), self.opn(b)) {
                    (BoolOp::Not, Opn::K(x), _) => Some(x == 0),
                    (BoolOp::And, Opn::K(x), Opn::K(y)) => Some(x != 0 && y != 0),
                    (BoolOp::Or, Opn::K(x), Opn::K(y)) => Some(x != 0 || y != 0),
                    (BoolOp::Xor, Opn::K(x), Opn::K(y)) => Some((x != 0) != (y != 0)),
                    (BoolOp::And, Opn::K(0), _) | (BoolOp::And, _, Opn::K(0)) => Some(false),
                    (BoolOp::Or, Opn::K(k), _) | (BoolOp::Or, _, Opn::K(k)) if k != 0 => Some(true),
                    _ => None,
                };
                if let Some(v) = v {
                    return self.make_const(pc, dst, i64::from(v));
                }
                let y = if op == BoolOp::Not {
                    Some(Opn::K(0))
                } else {
                    self.kopn(b)
                };
                if let (Some(x), Some(y)) = (self.kopn(a), y) {
                    self.cse(pc, Some(dst), Key::Bool(op, x, y));
                }
            }
            Inst::Index {
                dst,
                base,
                pos,
                stride,
            } => {
                if self.fold_index(pc) {
                    return;
                }
                if let (Opn::R(x), Opn::K(0)) = (self.opn(base), self.opn(pos))
                    && self.alias_ok(pc, dst, x)
                {
                    return;
                }
                if let (Some(x), Some(y)) = (self.kopn(base), self.kopn(pos)) {
                    self.cse(pc, Some(dst), Key::Index(x, y, stride));
                }
            }
            Inst::CondBr { c, t, f } => {
                if let Opn::K(v) = self.opn(c) {
                    self.code[pc] = Inst::Br {
                        to: if v != 0 { t } else { f },
                    };
                    self.changed = true;
                }
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
                    self.changed = true;
                }
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
                    self.changed = true;
                    return;
                }
                // Operands the check does not read do not tell checks apart.
                let (nl, nh) = match kind {
                    Null => (Opn::K(0), Opn::K(0)),
                    Length => (nl, Opn::K(0)),
                    _ => (nl, nh),
                };
                if self.key_ok(nv) && self.key_ok(nl) && self.key_ok(nh) {
                    self.cse(pc, None, Key::Check(kind, nv, nl, nh, what));
                }
            }
            Inst::Load { dst, addr, w } if w != Width::Sig => {
                let a = self.opn(addr);
                let n = w.bytes() as usize;
                if let Some(b) = self.const_bytes(a, n) {
                    let mut x = [0u8; 8];
                    x[..n].copy_from_slice(b);
                    // A 4-byte load sign-extends (narrow integer elements, #415).
                    let v = if n == 4 {
                        i64::from(i32::from_le_bytes([x[0], x[1], x[2], x[3]]))
                    } else {
                        u64::from_le_bytes(x) as i64
                    };
                    return self.make_const(pc, dst, v);
                }
                if let Some(a) = self.kopn(addr) {
                    self.cse(pc, Some(dst), Key::Load(a, w));
                }
            }
            Inst::DopeLen { dst, dope, dim } => {
                if let Some((l, r, dir)) = self.const_dim(self.opn(dope), dim)
                    && let Some(n) = dim_len(l, r, dir == 1).filter(|&n| n <= i64::MAX as u64)
                {
                    return self.make_const(pc, dst, n as i64);
                }
                if let Some(d) = self.kopn(dope) {
                    self.cse(pc, Some(dst), Key::Dope(d, dim));
                }
            }
            Inst::Pos {
                dst,
                dope,
                dim,
                idx,
                what,
            } => {
                if let (Some((l, r, dir)), Opn::K(i)) =
                    (self.const_dim(self.opn(dope), dim), self.opn(idx))
                {
                    let pos = match dir {
                        0 if l <= i && i <= r => Some(i.wrapping_sub(l)),
                        1 if r <= i && i <= l => Some(l.wrapping_sub(i)),
                        _ => None,
                    };
                    if let Some(p) = pos {
                        return self.make_const(pc, dst, p);
                    }
                }
                if let (Some(d), Some(i)) = (self.kopn(dope), self.kopn(idx)) {
                    self.cse(pc, Some(dst), Key::Pos(d, dim, i, what));
                }
            }
            Inst::LayoutOf { dst, ty, desc } => {
                if let Some(d) = self.kopn(desc) {
                    self.cse(pc, Some(dst), Key::Layout(ty, d));
                }
            }
            Inst::Field {
                dst,
                base,
                layout,
                n,
            } => {
                // The layout of a type without a descriptor is the same everywhere: the
                // field is a constant offset.
                if let Opn::R(l) = self.opn(layout)
                    && let Some(&ty) = self.slay.get(&l)
                    && let Some(off) = self.static_offset(ty, n)
                {
                    let c = self.const_reg(self.cur, pc, off as i64);
                    self.code[pc] = Inst::Index {
                        dst,
                        base,
                        pos: c,
                        stride: 1,
                    };
                    self.changed = true;
                    return self.simplify(pc);
                }
                if let (Some(b), Some(l)) = (self.kopn(base), self.kopn(layout)) {
                    self.cse(pc, Some(dst), Key::Field(b, l, n));
                }
            }
            _ => {}
        }
    }

    /// Remove pure definitions nobody reads.
    fn dce(&mut self) {
        let mut uses = vec![0u32; self.defs.len()];
        let rep = &self.rep;
        let res = |mut r: Reg| {
            while let Some(&x) = rep.get(r.0 as usize) {
                if x == r.0 {
                    break;
                }
                r = Reg(x);
            }
            r
        };
        // Follow the renames and count the readers in one pass.
        let rename = self.nrep > 0;
        for i in &mut self.code {
            if is_dead(i) {
                continue;
            }
            i.visit_mut(&mut |r, role, _| {
                if role == Role::Use {
                    if rename {
                        *r = res(*r);
                    }
                    if let Some(u) = uses.get_mut(r.0 as usize) {
                        *u += 1;
                    }
                }
            });
            if let Some(l) = i.list() {
                for r in self
                    .operands
                    .get_mut(l.start as usize..(l.start + l.len) as usize)
                    .unwrap_or(&mut [])
                {
                    if rename {
                        *r = res(*r);
                    }
                    if let Some(u) = uses.get_mut(r.0 as usize) {
                        *u += 1;
                    }
                }
            }
        }
        // One backward sweep removes whole chains: a user comes after its operands.
        for pc in (0..self.code.len()).rev() {
            let i = self.code[pc];
            let dst = match i {
                Inst::LayoutOf { dst, .. } if self.slay.contains_key(&dst) => Some(dst),
                _ => removable_dst(&i),
            };
            let Some(dst) = dst else {
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
            self.changed = true;
        }
        self.uses = uses;
    }

    /// Remove the dead instructions and add the inserted ones, with their locations; the new
    /// block ranges.
    #[allow(clippy::needless_range_loop)]
    fn compact(&mut self, loc: &mut Vec<StmtId>) -> Vec<(u32, u32)> {
        self.inserts.sort_by_key(|x| x.0);
        let mut ins = self.inserts.iter().peekable();
        let mut code = Vec::with_capacity(self.code.len() + self.inserts.len());
        let mut locs = Vec::with_capacity(code.capacity());
        let mut blocks = Vec::with_capacity(self.blocks.len());
        for &(s, e) in &self.blocks {
            let start = code.len() as u32;
            for pc in s as usize..e as usize {
                while let Some(&(_, i)) = ins.next_if(|x| x.0 == pc) {
                    // Skip a constant nobody ended up reading.
                    if let Inst::Const { dst, .. } = i
                        && self.uses.get(dst.0 as usize) == Some(&0)
                    {
                        continue;
                    }
                    code.push(i);
                    locs.push(loc[pc]);
                }
                if !is_dead(&self.code[pc]) {
                    code.push(self.code[pc]);
                    locs.push(loc[pc]);
                }
            }
            blocks.push((start, code.len() as u32));
        }
        self.code = code;
        *loc = locs;
        blocks
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
        Inst::Load { dst, w, .. } if w != Width::Sig => Some(dst),
        Inst::Field { dst, .. } => Some(dst),
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
