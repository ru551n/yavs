//! The bodies of member processes, from the elaborated HIR to region operations on two-state
//! words: a logic value of `len` elements is `ceil(len / 64)` words (element 0 in bit 0, the bits
//! above `len` zero), an integer or an enumeration position one word. Metavalues read as 0: a
//! literal's are counted per site, a variable's missing initial value per element. Anything else
//! is a reason to leave the process in the event kernel, never a guess.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use yavs_elab::{Bounds, Elaborated, LeafId, ProcInst, Shape};
use yavs_sem::hir::{
    Actual, AggBounds, AttrPrefix, Choice, ChoiceSet, ExprKind, LoopScheme, PredefAttr, Rhs,
    Sensitivity, StmtKind, Target, Waveform,
};
use yavs_sem::implicit::Family;
use yavs_sem::stat::{ConstValue, Elems};
use yavs_sem::types::{Constraint, Direction, RangeC, TypeKind};
use yavs_sem::{Db, DeclId, ExprId, StmtId, SubtypeId, TypeId};

use crate::codegen::{mask, words};

/// Words of the region's state: the current value of a kernel signal (all the leaves the kernel
/// keeps in it, each at its kernel offset), a slot of the process (a leaf of a variable, a
/// temporary), numbered per body, or the region's edge word (1 when the clock edge happened in
/// this wake; read by processes with an asynchronous reset).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum Ref {
    Sig(u32),
    Loc(u32),
    Edge,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Gate {
    And,
    Or,
    Xor,
    Nand,
    Nor,
    Xnor,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Cmp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Arith {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Rem,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ShiftK {
    Sll,
    Srl,
    Sra,
    Rol,
    Ror,
}

/// What a check says when it fails: the texts of the normal mode (`ErrorKind::Runtime`).
#[derive(Clone, Debug)]
pub(crate) enum Check {
    /// `result of X op Y cannot be represented as T`, or `division by zero`.
    Arith { op: Arith, ty: String },
    /// `value V outside of T range B for W`.
    Range {
        ty: String,
        lo: i64,
        hi: i64,
        down: bool,
        what: String,
    },
    /// `index V outside of range B for W`.
    Index { b: Bounds, what: String },
}

impl Check {
    /// The message for the operands the code stored (`a`, and `b` for arithmetic).
    pub(crate) fn message(&self, a: i64, b: i64) -> String {
        let range = |lo: i64, hi: i64, down: bool| {
            if down {
                format!("{hi} downto {lo}")
            } else {
                format!("{lo} to {hi}")
            }
        };
        match self {
            Check::Arith { op, .. }
                if b == 0 && matches!(op, Arith::Div | Arith::Mod | Arith::Rem) =>
            {
                "division by zero".into()
            }
            Check::Arith { op, ty } => {
                let sym = match op {
                    Arith::Add => "+",
                    Arith::Sub => "-",
                    Arith::Mul => "*",
                    Arith::Div => "/",
                    Arith::Mod => "mod",
                    Arith::Rem => "rem",
                };
                format!("result of {a} {sym} {b} cannot be represented as {ty}")
            }
            Check::Range {
                ty,
                lo,
                hi,
                down,
                what,
            } => format!(
                "value {a} outside of {ty} range {} for {what}",
                range(*lo, *hi, *down)
            ),
            Check::Index { b: x, what } => {
                let (lo, hi) = x.low_high();
                format!(
                    "index {a} outside of range {} for {what}",
                    range(lo, hi, x.dir == Direction::Downto)
                )
            }
        }
    }
}

/// One operation on words. Every operand is read before the result is written, so a result may
/// be an operand. `len` is in elements; a 1-bit value is a boolean or a scalar logic value.
#[derive(Clone, Debug)]
pub(crate) enum Op {
    Copy {
        dst: Ref,
        src: Ref,
        len: u32,
    },
    Lit {
        dst: Ref,
        val: Box<[u64]>,
    },
    /// `dst := dst & !mask | src & mask`, word by word: the leaves of a record at the same
    /// places of two signals, copied together.
    Blend {
        dst: Ref,
        src: Ref,
        mask: Box<[u64]>,
    },
    Gate {
        g: Gate,
        dst: Ref,
        a: Ref,
        b: Ref,
        len: u32,
    },
    Not {
        dst: Ref,
        a: Ref,
        len: u32,
    },
    /// Elements `lo .. lo + len` of `src` (of `full` elements) as a value of `len` elements.
    Extract {
        dst: Ref,
        src: Ref,
        lo: u32,
        len: u32,
        full: u32,
    },
    /// `src`'s `len` elements into elements `lo .. lo + len` of `dst`.
    Insert {
        dst: Ref,
        src: Ref,
        lo: u32,
        len: u32,
    },
    /// `hi & lo` (the left operand is the high part): `lo_len` elements of `lo` below `hi`, `len`
    /// in all.
    Cat {
        dst: Ref,
        hi: Ref,
        lo: Ref,
        lo_len: u32,
        len: u32,
    },
    /// `numeric_std` `+` or `-` of two vectors of `len` elements, modulo `2 ** len`.
    Add {
        dst: Ref,
        a: Ref,
        b: Ref,
        len: u32,
        sub: bool,
    },
    /// The product of two vectors of `len` elements, modulo `2 ** len`.
    Mul {
        dst: Ref,
        a: Ref,
        b: Ref,
        len: u32,
    },
    /// `/`, `mod` or `rem` of two vectors of `len` (at most 64) elements; a zero divisor fails
    /// check `site`.
    VDiv {
        op: Arith,
        signed: bool,
        dst: Ref,
        a: Ref,
        b: Ref,
        len: u32,
        site: u32,
    },
    /// `a = b` (or `/=`) into a boolean.
    VecEq {
        dst: Ref,
        a: Ref,
        b: Ref,
        len: u32,
        ne: bool,
    },
    /// A numeric comparison of two vectors of `len` elements into a boolean.
    VCmp {
        cmp: Cmp,
        signed: bool,
        dst: Ref,
        a: Ref,
        b: Ref,
        len: u32,
    },
    /// `src` of `from` elements zero- or sign-extended, or cut, to `to` elements.
    Ext {
        dst: Ref,
        src: Ref,
        from: u32,
        to: u32,
        signed: bool,
    },
    /// A shift or rotation of `a` (at most 64 elements) by the integer `n`, at least 0.
    Shift {
        k: ShiftK,
        dst: Ref,
        a: Ref,
        len: u32,
        n: Ref,
    },
    /// The gate of all elements of `a` into a boolean.
    Reduce {
        g: Gate,
        dst: Ref,
        a: Ref,
        len: u32,
    },
    /// The vector `a` (at most 64 elements) as an integer.
    ToInt {
        dst: Ref,
        a: Ref,
        len: u32,
        signed: bool,
    },
    /// The low `len` bits of the integer `a` (two's complement).
    FromInt {
        dst: Ref,
        a: Ref,
        len: u32,
    },
    /// The boolean `a` in every one of `len` elements.
    Splat {
        dst: Ref,
        a: Ref,
        len: u32,
    },
    /// `c ? a : b` on values of `len` elements (an integer: 64).
    Select {
        dst: Ref,
        c: Ref,
        a: Ref,
        b: Ref,
        len: u32,
    },
    IntLit {
        dst: Ref,
        v: i64,
    },
    IntCmp {
        op: Cmp,
        dst: Ref,
        a: Ref,
        b: Ref,
    },
    /// Integer arithmetic, the result checked to lie in `lo ..= hi` (its type) at check `site`.
    Int {
        op: Arith,
        dst: Ref,
        a: Ref,
        b: Ref,
        lo: i64,
        hi: i64,
        site: u32,
    },
    /// `a` checked to lie in `lo ..= hi`.
    Range {
        a: Ref,
        lo: i64,
        hi: i64,
        site: u32,
    },
    /// The `w` elements (`w / stride` array elements) from array element `idx` on (its
    /// rightmost, for a slice) of the array of `n` elements, each `stride` wide, from element `lo`
    /// of `src`; index range from `left` (`down`: descending), checked.
    IndexRead {
        dst: Ref,
        src: Ref,
        lo: u32,
        n: u32,
        stride: u32,
        w: u32,
        idx: Ref,
        left: i64,
        down: bool,
        site: u32,
    },
    /// The `w` elements of `src` into elements from `idx` on of such an array of `dst`.
    IndexWrite {
        dst: Ref,
        src: Ref,
        lo: u32,
        n: u32,
        stride: u32,
        w: u32,
        idx: Ref,
        left: i64,
        down: bool,
        site: u32,
    },
    If {
        cond: Ref,
        then: Vec<Op>,
        else_: Vec<Op>,
    },
}

impl Op {
    fn dst_mut(&mut self) -> Option<&mut Ref> {
        match self {
            Op::Copy { dst, .. }
            | Op::Lit { dst, .. }
            | Op::Gate { dst, .. }
            | Op::Not { dst, .. }
            | Op::Extract { dst, .. }
            | Op::Cat { dst, .. }
            | Op::Add { dst, .. }
            | Op::Mul { dst, .. }
            | Op::VDiv { dst, .. }
            | Op::VecEq { dst, .. }
            | Op::VCmp { dst, .. }
            | Op::Ext { dst, .. }
            | Op::Shift { dst, .. }
            | Op::Reduce { dst, .. }
            | Op::ToInt { dst, .. }
            | Op::FromInt { dst, .. }
            | Op::Splat { dst, .. }
            | Op::Select { dst, .. }
            | Op::IntLit { dst, .. }
            | Op::IntCmp { dst, .. }
            | Op::Int { dst, .. }
            | Op::IndexRead { dst, .. } => Some(dst),
            // These read what they write.
            Op::Blend { .. }
            | Op::Insert { .. }
            | Op::IndexWrite { .. }
            | Op::Range { .. }
            | Op::If { .. } => None,
        }
    }

    /// The operation with every reference mapped: `s` for what it reads, `d` for what it
    /// writes (an operation that reads what it writes reads it through `d` too), check sites
    /// through `site`.
    pub(crate) fn map(
        &self,
        s: &dyn Fn(Ref) -> Ref,
        d: &dyn Fn(Ref) -> Ref,
        site: &dyn Fn(u32) -> u32,
    ) -> Op {
        let mut o = self.clone();
        match &mut o {
            Op::Copy { dst, src, .. }
            | Op::Extract { dst, src, .. }
            | Op::Ext { dst, src, .. }
            | Op::Blend { dst, src, .. }
            | Op::Insert { dst, src, .. } => {
                *dst = d(*dst);
                *src = s(*src);
            }
            Op::Lit { dst, .. } | Op::IntLit { dst, .. } => *dst = d(*dst),
            Op::Gate { dst, a, b, .. }
            | Op::Add { dst, a, b, .. }
            | Op::Mul { dst, a, b, .. }
            | Op::VecEq { dst, a, b, .. }
            | Op::VCmp { dst, a, b, .. }
            | Op::IntCmp { dst, a, b, .. } => {
                *dst = d(*dst);
                *a = s(*a);
                *b = s(*b);
            }
            Op::VDiv {
                dst, a, b, site: x, ..
            }
            | Op::Int {
                dst, a, b, site: x, ..
            } => {
                *dst = d(*dst);
                *a = s(*a);
                *b = s(*b);
                *x = site(*x);
            }
            Op::Not { dst, a, .. }
            | Op::Reduce { dst, a, .. }
            | Op::ToInt { dst, a, .. }
            | Op::FromInt { dst, a, .. }
            | Op::Splat { dst, a, .. } => {
                *dst = d(*dst);
                *a = s(*a);
            }
            Op::Shift { dst, a, n, .. } => {
                *dst = d(*dst);
                *a = s(*a);
                *n = s(*n);
            }
            Op::Cat { dst, hi, lo, .. } => {
                *dst = d(*dst);
                *hi = s(*hi);
                *lo = s(*lo);
            }
            Op::Select { dst, c, a, b, .. } => {
                *dst = d(*dst);
                *c = s(*c);
                *a = s(*a);
                *b = s(*b);
            }
            Op::Range { a, site: x, .. } => {
                *a = s(*a);
                *x = site(*x);
            }
            Op::IndexRead {
                dst,
                src,
                idx,
                site: x,
                ..
            }
            | Op::IndexWrite {
                dst,
                src,
                idx,
                site: x,
                ..
            } => {
                *dst = d(*dst);
                *src = s(*src);
                *idx = s(*idx);
                *x = site(*x);
            }
            Op::If { cond, then, else_ } => {
                *cond = s(*cond);
                for op in then.iter_mut().chain(else_.iter_mut()) {
                    *op = op.map(s, d, site);
                }
            }
        }
        o
    }

    /// The operation can stop the run (a check).
    fn can_fail(&self) -> bool {
        match self {
            Op::Int { .. }
            | Op::Range { .. }
            | Op::VDiv { .. }
            | Op::IndexRead { .. }
            | Op::IndexWrite { .. } => true,
            Op::If { then, else_, .. } => then.iter().chain(else_).any(Op::can_fail),
            _ => false,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Kind {
    Clocked,
    Comb,
}

/// The clock edge of a clocked member.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Clock {
    pub leaf: LeafId,
    pub elem: u32,
    pub rising: bool,
    pub legacy: bool,
}

/// A lowered member body.
pub(crate) struct Body {
    pub kind: Kind,
    pub clock: Option<Clock>,
    /// The asynchronous reset of a clocked member (element and `std_ulogic` code): its code
    /// tests the edge word itself and runs at every wake.
    pub reset: Option<(LeafId, u32, u8)>,
    pub ops: Vec<Op>,
    /// Run once at initialisation: the initial values of variables.
    pub init_ops: Vec<Op>,
    /// Signal leaves read and written.
    pub reads: BTreeSet<LeafId>,
    pub writes: BTreeSet<LeafId>,
    /// The elements read, per leaf (`(leaf, first, count)`, from the leaf's element 0).
    pub rparts: Vec<(LeafId, u32, u32)>,
    /// Words of `Loc` slots used, `0 .. locs`.
    pub locs: u32,
    /// Initial values of slots other than 0: integer variables start at their `'left`.
    pub init: Vec<(u32, u64)>,
    /// The words of the variables (first, count): in a clocked process, state kept from one
    /// edge to the next.
    pub vars: Vec<(u32, u32)>,
    /// Asserts and reports left out (not evaluated in cycle mode).
    pub dropped: u32,
    /// Literals with a metavalue (`'U'`, `'X'`, `'W'`, `'-'`), read as 0.
    pub metas: u32,
    /// Elements of logic variables without an initial value (`'U'`), which start at 0.
    pub uninit: u64,
    /// The checks of the body, numbered by `site`.
    pub checks: Vec<Check>,
}

/// The index ranges of a logic array, the outermost first, nested arrays of logic elements
/// included (an array of vectors, of arrays of vectors).
#[derive(Clone, Copy, Debug)]
struct Dims {
    b: [Bounds; Dims::MAX],
    n: u8,
}

impl Dims {
    const MAX: usize = 4;
    const NONE: Dims = Dims {
        b: [Bounds {
            left: 0,
            dir: Direction::To,
            right: -1,
        }; Dims::MAX],
        n: 0,
    };

    fn first(&self) -> Option<Bounds> {
        (self.n > 0).then_some(self.b[0])
    }

    /// The index ranges of one element.
    fn rest(&self) -> Dims {
        let mut d = Dims::NONE;
        if self.n > 0 {
            d.b[..self.n as usize - 1].copy_from_slice(&self.b[1..self.n as usize]);
            d.n = self.n - 1;
        }
        d
    }

    /// The logic elements of one element of the outermost range.
    fn elem_len(&self) -> u32 {
        (self.b[1..self.n.max(1) as usize].iter())
            .map(|b| b.len() as u32)
            .product()
    }

    /// `outer` around `inner`; `None` when too deep.
    fn wrap(outer: Bounds, inner: Dims) -> Option<Dims> {
        if inner.n as usize >= Dims::MAX {
            return None;
        }
        let mut d = Dims::NONE;
        d.b[0] = outer;
        d.b[1..=inner.n as usize].copy_from_slice(&inner.b[..inner.n as usize]);
        d.n = inner.n + 1;
        Some(d)
    }

    /// The same, the outermost range replaced.
    fn with_first(mut self, b: Bounds) -> Dims {
        self.b[0] = b;
        self
    }
}

/// A value or a place: a part of a run of words, an integer, or a record of them.
#[derive(Clone, Debug)]
enum Val {
    Part(Part),
    Int(IntV),
    Rec(Vec<Val>),
}

#[derive(Clone, Copy, Debug)]
struct Part {
    r: Ref,
    /// Elements `lo .. lo + len` of words holding `full` elements.
    lo: u32,
    len: u32,
    full: u32,
    /// The index ranges of an array part, the outermost first (none: a scalar, or a value with
    /// no name); an array of vectors has two.
    dims: Dims,
    /// A temporary made for this value alone: the operation that made it may write elsewhere.
    fresh: bool,
    /// The signal leaf the part is of (reads and writes of the body are counted by leaf).
    leaf: Option<LeafId>,
    /// Elements of the array from element `lo`, chosen at run time: the index of the first (the
    /// rightmost of a slice), the array's index range and the width of one of its elements; the
    /// part is `len` wide.
    at: Option<(Ref, Bounds, u32)>,
    /// The object a name starts from (messages of index checks).
    obj: Option<DeclId>,
    /// A literal with a metavalue element: its `std_ulogic` codes (`Lx::lits`), since no
    /// two-state comparison means what the comparison with it means in VHDL.
    lit: Option<u32>,
}

#[derive(Clone, Copy, Debug)]
struct IntV {
    r: Ref,
    /// The values it can hold: its subtype's range for a place, the result type's for an
    /// operation, the value for a literal.
    lo: i64,
    hi: i64,
    down: bool,
    /// A place: its declared subtype and object, for the range check of an assignment.
    place: Option<(SubtypeId, DeclId)>,
    fresh: bool,
    leaf: Option<LeafId>,
}

type R<T> = Result<T, String>;

/// The elements a path through a body certainly assigns, or may assign: per kernel signal, a
/// bit per element.
#[derive(Clone, Default)]
struct Assigned(BTreeMap<u32, Vec<u64>>);

impl Assigned {
    fn add(&mut self, k: u32, lo: u32, len: u32) {
        let w = self.0.entry(k).or_default();
        let need = (lo + len).div_ceil(64) as usize;
        if w.len() < need {
            w.resize(need, 0);
        }
        set_bits(w, lo, len);
    }

    /// Only what both assign.
    fn meet(&mut self, o: &Assigned) {
        self.0.retain(|k, w| match o.0.get(k) {
            Some(x) => {
                for (i, a) in w.iter_mut().enumerate() {
                    *a &= x.get(i).copied().unwrap_or(0);
                }
                true
            }
            None => false,
        });
    }

    /// A kernel signal with an element in `self` that `o` does not have.
    fn beyond(&self, o: &Assigned) -> Option<u32> {
        self.0.iter().find_map(|(k, w)| {
            let x = o.0.get(k);
            (w.iter().enumerate())
                .any(|(i, a)| a & !x.and_then(|x| x.get(i).copied()).unwrap_or(0) != 0)
                .then_some(*k)
        })
    }
}

/// What a branch of an `if`, `case` or a conditional or selected assignment does.
#[derive(Clone, Copy)]
enum Br<'b> {
    Stmts(&'b [StmtId]),
    Wave(&'b Target, &'b Waveform),
    Var(&'b Target, ExprId),
}

/// Most statement instances an unrolled loop makes per process.
const UNROLL_MAX: u32 = 1 << 16;

/// Lower process `pi` if its shape and every construct of its body are in the subset.
pub(crate) fn lower(db: &Db, e: &Elaborated, pi: usize) -> R<Body> {
    let p = &e.procs[pi];
    if p.actual.is_some() {
        return Err("a port expression actual is not supported in cycle mode yet".into());
    }
    if p.postponed {
        return Err("a postponed process cannot run cycle-based".into());
    }
    let mut lx = Lx {
        db,
        e,
        p,
        vars: HashMap::new(),
        consts: HashMap::new(),
        ops: Vec::new(),
        init_ops: Vec::new(),
        reads: BTreeSet::new(),
        rparts: Vec::new(),
        writes: BTreeSet::new(),
        locs: 0,
        init: Vec::new(),
        var_words: Vec::new(),
        dropped: 0,
        metas: 0,
        uninit: 0,
        checks: Vec::new(),
        must: Assigned::default(),
        may: Assigned::default(),
        unrolled: 0,
        lits: Vec::new(),
    };
    match &db.stmts[p.stmt].kind {
        StmtKind::Process {
            sensitivity,
            body,
            scope,
            ..
        } => lx.process(sensitivity, body, *scope),
        StmtKind::ConcSignalAssign {
            postponed: false,
            guarded: false,
            target,
            delay,
            rhs,
            ..
        } => lx.conc_assign(target, delay, rhs),
        StmtKind::ConcSignalAssign { .. } => {
            Err("a guarded or postponed concurrent assignment cannot run cycle-based".into())
        }
        // Simulation-only, as in processes: not evaluated, and it reads nothing.
        StmtKind::ConcAssert {
            postponed: false, ..
        } => {
            lx.dropped += 1;
            Ok(lx.body(Kind::Comb, None, None))
        }
        StmtKind::ConcProcCall { .. } => {
            Err("a concurrent procedure call is not supported in cycle mode yet".into())
        }
        _ => Err("a concurrent statement of this kind is not supported in cycle mode yet".into()),
    }
}

struct Lx<'a> {
    db: &'a Db,
    e: &'a Elaborated,
    p: &'a ProcInst,
    /// The leaves of each process variable met so far.
    vars: HashMap<DeclId, Val>,
    /// The parameters of the loops being unrolled, with their value in this iteration.
    consts: HashMap<DeclId, i64>,
    ops: Vec<Op>,
    init_ops: Vec<Op>,
    reads: BTreeSet<LeafId>,
    rparts: Vec<(LeafId, u32, u32)>,
    writes: BTreeSet<LeafId>,
    locs: u32,
    init: Vec<(u32, u64)>,
    var_words: Vec<(u32, u32)>,
    dropped: u32,
    metas: u32,
    uninit: u64,
    checks: Vec<Check>,
    /// The signal elements every path so far assigns, and those some path may assign.
    must: Assigned,
    may: Assigned,
    unrolled: u32,
    /// The codes of the literals with a metavalue (`Part::lit`).
    lits: Vec<Vec<u8>>,
}

impl Lx<'_> {
    fn body(self, kind: Kind, clock: Option<Clock>, reset: Option<(LeafId, u32, u8)>) -> Body {
        Body {
            kind,
            clock,
            reset,
            ops: self.ops,
            init_ops: self.init_ops,
            reads: self.reads,
            writes: self.writes,
            rparts: self.rparts,
            locs: self.locs,
            init: self.init,
            vars: self.var_words,
            dropped: self.dropped,
            metas: self.metas,
            uninit: self.uninit,
            checks: self.checks,
        }
    }

    fn process(
        mut self,
        sensitivity: &Sensitivity,
        body: &[StmtId],
        scope: yavs_sem::ScopeId,
    ) -> R<Body> {
        let db = self.db;
        if let Some(t) = self.p.trigger {
            let clock = Clock {
                leaf: t.leaf,
                elem: t.elem,
                rising: t.rising,
                legacy: t.legacy,
            };
            if t.wait {
                // `wait until <edge>;` first: the rest runs at every edge.
                let [w, rest @ ..] = body else {
                    return Err("an empty clocked process".into());
                };
                let StmtKind::Wait { until: Some(c), .. } = db.stmts[*w].kind else {
                    return Err(
                        "a `wait until` process whose wait is not its first statement is not \
                         supported in cycle mode yet"
                            .into(),
                    );
                };
                self.edge_body(c, rest)?;
                return Ok(self.body(Kind::Clocked, Some(clock), None));
            }
            let [s] = body else {
                return Err("a clocked process with statements outside its edge test".into());
            };
            let StmtKind::If {
                branches,
                else_: None,
            } = &db.stmts[*s].kind
            else {
                return Err("a clocked process with an `else` branch".into());
            };
            match (&branches[..], t.reset) {
                ([(cond, inner)], None) => {
                    self.edge_body(*cond, inner)?;
                    return Ok(self.body(Kind::Clocked, Some(clock), None));
                }
                ([(r, a), (cond, b)], Some(reset)) => {
                    // `if <reset> then A elsif <edge> then B`: the edge word decides B.
                    let c = self.cond(*r)?;
                    let outer = std::mem::take(&mut self.ops);
                    self.stmts(a)?;
                    let then = std::mem::take(&mut self.ops);
                    self.edge_body(*cond, b)?;
                    let at_edge = std::mem::replace(&mut self.ops, outer);
                    self.ops.push(Op::If {
                        cond: c,
                        then,
                        else_: vec![Op::If {
                            cond: Ref::Edge,
                            then: at_edge,
                            else_: Vec::new(),
                        }],
                    });
                    return Ok(self.body(Kind::Clocked, Some(clock), Some(reset)));
                }
                _ => return Err("a clocked process with an `elsif` branch".into()),
            }
        }
        if matches!(sensitivity, Sensitivity::None) {
            return Err("a process with wait statements runs in the event kernel".into());
        }
        // What an activation does must be a function of the signals it reads: no state kept in
        // variables from one activation to the next.
        // Asserts and reports are not evaluated in the region, so they do not count here.
        let kept: Vec<StmtId> = (body.iter().copied())
            .filter(|&s| {
                !matches!(
                    db.stmts[s].kind,
                    StmtKind::Assert { .. } | StmtKind::Report { .. }
                )
            })
            .collect();
        if db.pure_comb_reads(scope, &kept).is_none() {
            return Err(
                "a combinational process that keeps state in a variable or does more than assign \
                 signals and variables"
                    .into(),
            );
        }
        self.stmts(body)?;
        // Cycle mode computes from every read; the normal mode only wakes on the list.
        if let Sensitivity::List(list) = sensitivity {
            let mut listed = BTreeSet::new();
            for l in list.iter() {
                let yavs_sem::hir::ObjRoot::Decl(d) = l.root else {
                    return Err("a sensitivity list naming an external name".into());
                };
                // A part names fewer leaves than its object: not taken yet (the check must not
                // count leaves the list does not name).
                if !l.path.is_empty() {
                    return Err(
                        "a sensitivity list naming part of a signal is not supported in cycle \
                         mode yet"
                            .into(),
                    );
                }
                let Some(o) = self.e.signal(self.p.scope, d) else {
                    return Err("a sensitivity list naming something that is not a signal".into());
                };
                let (a, z) = self.e.nets.obj(o).leaves;
                listed.extend((a.0..z.0).map(LeafId));
            }
            if let Some(l) = self.reads.iter().find(|l| !listed.contains(l)) {
                return Err(format!(
                    "its sensitivity list does not name {}, which it reads",
                    self.e.leaf_name(*l)
                ));
            }
        }
        self.latch_free()?;
        // A combinational variable is assigned before any read (`pure_comb_reads`): its start
        // value is never seen.
        self.uninit = 0;
        self.init_ops.clear();
        Ok(self.body(Kind::Comb, None, None))
    }

    /// The body of a clocked process under its edge condition `cond`: the edge itself, or the
    /// edge `and` a condition that decides whether the body runs at the edge.
    fn edge_body(&mut self, cond: ExprId, body: &[StmtId]) -> R<()> {
        match self.edge_rest(cond)? {
            None => self.stmts(body),
            Some(c) => self.if_chain(&[(Some(c), Br::Stmts(body))]),
        }
    }

    /// What of an edge condition is not the edge (as the elaborator found it): `None` for the
    /// edge alone.
    fn edge_rest(&self, cond: ExprId) -> R<Option<ExprId>> {
        let db = self.db;
        let edge = |x: ExprId| {
            db.edge_shape(x).is_some()
                || matches!(db.level_of(x), Some((_, 2 | 3)))
                    && self.p.trigger.is_some_and(|t| t.wait)
        };
        if edge(cond) {
            return Ok(None);
        }
        if let ExprKind::Call { callee, args, .. } = &db.exprs[cond].kind
            && let [a, b] = &args[..]
            && let (Actual::Expr(a), Actual::Expr(b)) = (&a.actual, &b.actual)
            && db.decls[*callee].implicit
            && db.decls[*callee].name.key().as_str() == "\"and\""
        {
            if edge(*a) {
                return Ok(Some(*b));
            }
            if edge(*b) {
                return Ok(Some(*a));
            }
        }
        Err(
            "a clocked process whose condition is more than the edge test and one more condition"
                .into(),
        )
    }

    /// A concurrent signal assignment: combinational (its sensitivity is everything it reads),
    /// or `q <= d when <edge>;` on the kernel's edge trigger.
    fn conc_assign(
        mut self,
        target: &Target,
        delay: &yavs_sem::hir::Delay,
        rhs: &Rhs<Waveform>,
    ) -> R<Body> {
        if matches!(delay, yavs_sem::hir::Delay::Inertial { reject: Some(_) }) {
            return Err("a `reject` limit has no meaning in cycle mode".into());
        }
        if let Some(t) = self.p.trigger {
            let Rhs::Conditional(alts) = rhs else {
                return Err("an edge-triggered assignment of another shape".into());
            };
            let [(w, Some(c))] = &alts[..] else {
                return Err("an edge-triggered assignment of another shape".into());
            };
            if t.reset.is_some() || t.wait || self.db.edge_shape(*c).is_none() {
                return Err("an edge-triggered assignment of another shape".into());
            }
            self.wave(target, w)?;
            let clock = Clock {
                leaf: t.leaf,
                elem: t.elem,
                rising: t.rising,
                legacy: t.legacy,
            };
            return Ok(self.body(Kind::Clocked, Some(clock), None));
        }
        self.signal_rhs(target, rhs)?;
        self.latch_free()?;
        Ok(self.body(Kind::Comb, None, None))
    }

    /// The conservative must-assign test of a combinational member (milestone 3's latch
    /// analysis replaces it): every signal element some path may assign is assigned on every
    /// path, so the process keeps no value between activations.
    fn latch_free(&self) -> R<()> {
        match self.may.beyond(&self.must) {
            None => Ok(()),
            Some(k) => {
                let leaf = self.e.nets.packs[k as usize].0;
                Err(format!(
                    "it may keep the value of {} (not assigned on every path: a latch)",
                    self.e.leaf_name(leaf)
                ))
            }
        }
    }

    fn loc(&mut self, words: u32) -> Ref {
        self.locs += words;
        Ref::Loc(self.locs - words)
    }

    fn tmp(&mut self, len: u32) -> Part {
        Part {
            r: self.loc(words(len)),
            lo: 0,
            len,
            full: len,
            dims: Dims::NONE,
            fresh: true,
            leaf: None,
            at: None,
            obj: None,
            lit: None,
        }
    }

    fn tmp_int(&mut self, lo: i64, hi: i64) -> IntV {
        IntV {
            r: self.loc(1),
            lo,
            hi,
            down: false,
            place: None,
            fresh: true,
            leaf: None,
        }
    }

    fn int_lit(&mut self, v: i64) -> IntV {
        let i = self.tmp_int(v, v);
        self.ops.push(Op::IntLit { dst: i.r, v });
        i
    }

    fn bool_lit(&mut self, b: bool) -> Part {
        let t = self.tmp(1);
        self.ops.push(Op::Lit {
            dst: t.r,
            val: Box::new([u64::from(b)]),
        });
        t
    }

    fn site(&mut self, c: Check) -> u32 {
        self.checks.push(c);
        self.checks.len() as u32 - 1
    }

    fn stmts(&mut self, body: &[StmtId]) -> R<()> {
        for &s in body {
            self.stmt(s)?;
        }
        Ok(())
    }

    fn stmt(&mut self, s: StmtId) -> R<()> {
        let db = self.db;
        match &db.stmts[s].kind {
            StmtKind::Null => Ok(()),
            StmtKind::VarAssign { target, rhs } => match rhs {
                Rhs::Simple(v) => self.var_assign(target, *v),
                Rhs::Conditional(alts) => {
                    let alts: Vec<_> = (alts.iter())
                        .map(|(v, c)| (*c, Br::Var(target, *v)))
                        .collect();
                    self.if_chain(&alts)
                }
                Rhs::Selected {
                    expr,
                    matching,
                    alts,
                } => {
                    let alts: Vec<_> = (alts.iter())
                        .map(|(v, cs)| (cs, Br::Var(target, *v)))
                        .collect();
                    self.case(*expr, *matching, &alts)
                }
            },
            StmtKind::SignalAssign { target, delay, rhs } => {
                if matches!(delay, yavs_sem::hir::Delay::Inertial { reject: Some(_) }) {
                    return Err("a `reject` limit has no meaning in cycle mode".into());
                }
                self.signal_rhs(target, rhs)
            }
            StmtKind::If { branches, else_ } => {
                let mut alts: Vec<_> = (branches.iter())
                    .map(|(c, b)| (Some(*c), Br::Stmts(b)))
                    .collect();
                if let Some(e) = else_ {
                    alts.push((None, Br::Stmts(e)));
                }
                self.if_chain(&alts)
            }
            StmtKind::Case {
                expr,
                matching,
                alts,
            } => {
                let alts: Vec<_> = (alts.iter()).map(|(cs, b)| (cs, Br::Stmts(b))).collect();
                self.case(*expr, *matching, &alts)
            }
            StmtKind::Loop {
                scheme: LoopScheme::For { param, range },
                body,
                ..
            } => self.for_loop(*param, range, body),
            StmtKind::Loop { .. } => Err(
                "a `while` loop or a loop without a scheme is not supported in cycle mode yet"
                    .into(),
            ),
            StmtKind::Next { .. } | StmtKind::Exit { .. } => {
                Err("`next` and `exit` are not supported in cycle mode yet".into())
            }
            StmtKind::Wait { .. } => Err("a wait statement runs in the event kernel".into()),
            // Simulation-only: compiled to nothing, counted for the report (the mode is for speed).
            StmtKind::Assert { .. } | StmtKind::Report { .. } => {
                self.dropped += 1;
                Ok(())
            }
            StmtKind::ProcCall(_) => {
                Err("a procedure call is not supported in cycle mode yet".into())
            }
            StmtKind::Force { .. } | StmtKind::Release { .. } => {
                Err("`force` and `release` cannot run cycle-based".into())
            }
            StmtKind::Return(_) => Err("a `return` statement in a process".into()),
            _ => Err("a statement that is not supported in cycle mode yet".into()),
        }
    }

    fn var_assign(&mut self, target: &Target, v: ExprId) -> R<()> {
        let dst = self.name(target.expr, true)?;
        if self.signal_rooted(&dst) {
            return Err("an assignment target that is not a variable".into());
        }
        let src = self.expr_h(v, Some(&dst))?;
        self.assign(&dst, src)
    }

    fn signal_rhs(&mut self, target: &Target, rhs: &Rhs<Waveform>) -> R<()> {
        match rhs {
            Rhs::Simple(w) => self.wave(target, w),
            Rhs::Conditional(alts) => {
                let alts: Vec<_> = (alts.iter())
                    .map(|(w, c)| (*c, Br::Wave(target, w)))
                    .collect();
                self.if_chain(&alts)
            }
            Rhs::Selected {
                expr,
                matching,
                alts,
            } => {
                let alts: Vec<_> = (alts.iter())
                    .map(|(w, cs)| (cs, Br::Wave(target, w)))
                    .collect();
                self.case(*expr, *matching, &alts)
            }
        }
    }

    fn wave(&mut self, target: &Target, w: &Waveform) -> R<()> {
        let Waveform::Elements(w) = w else {
            // `unaffected`: nothing assigned.
            return Ok(());
        };
        let [el] = &w[..] else {
            return Err("a waveform of several elements needs time inside a cycle".into());
        };
        let (Some(v), None) = (el.value, el.after) else {
            return Err("a delayed assignment (`after`) needs time inside a cycle".into());
        };
        let dst = self.name(target.expr, true)?;
        if !self.signal_rooted(&dst) {
            return Err("a signal assignment to a signal parameter".into());
        }
        let src = self.expr_h(v, Some(&dst))?;
        self.assign(&dst, src)
    }

    fn branch(&mut self, b: Br) -> R<()> {
        match b {
            Br::Stmts(s) => self.stmts(s),
            Br::Wave(t, w) => self.wave(t, w),
            Br::Var(t, x) => self.var_assign(t, x),
        }
    }

    /// A boolean condition known during elaboration.
    fn fold_bool(&self, c: ExprId) -> Option<bool> {
        let t = self.db.base(self.db.exprs[c].subtype);
        if Some(t) != self.db.well_known.boolean {
            return None;
        }
        match self.fold(c)? {
            ConstValue::Enum(p) => Some(p == 1),
            _ => None,
        }
    }

    /// `if c0 then b0 elsif c1 then b1 ... else bn end if` (a condition `None` is the `else`) as
    /// nested `Op::If`, each condition evaluated only when the ones before it are false. A
    /// condition known during elaboration takes its branch alone.
    fn if_chain(&mut self, alts: &[(Option<ExprId>, Br)]) -> R<()> {
        let Some(((c, b), rest)) = alts.split_first() else {
            return Ok(());
        };
        let Some(c) = c else {
            return self.branch(*b);
        };
        if let Some(v) = self.fold_bool(*c) {
            return if v {
                self.branch(*b)
            } else {
                self.if_chain(rest)
            };
        }
        let cond = self.cond(*c)?;
        let saved = self.must.clone();
        let outer = std::mem::take(&mut self.ops);
        self.branch(*b)?;
        let then = std::mem::take(&mut self.ops);
        let then_must = std::mem::replace(&mut self.must, saved);
        self.if_chain(rest)?;
        let else_ = std::mem::replace(&mut self.ops, outer);
        self.must.meet(&then_must);
        self.ops.push(Op::If { cond, then, else_ });
        Ok(())
    }

    /// The branches `alts` behind conditions already computed (`None`: the last, taken when
    /// no other is).
    fn ref_chain(&mut self, alts: &[(Option<Ref>, Br)]) -> R<()> {
        let Some(((c, b), rest)) = alts.split_first() else {
            return Ok(());
        };
        let Some(cond) = c else {
            return self.branch(*b);
        };
        let saved = self.must.clone();
        let outer = std::mem::take(&mut self.ops);
        self.branch(*b)?;
        let then = std::mem::take(&mut self.ops);
        let then_must = std::mem::replace(&mut self.must, saved);
        self.ref_chain(rest)?;
        let else_ = std::mem::replace(&mut self.ops, outer);
        self.must.meet(&then_must);
        self.ops.push(Op::If {
            cond: *cond,
            then,
            else_,
        });
        Ok(())
    }

    /// `case` (and a selected assignment) as a chain of tests of the choices; the last
    /// alternative, or `others`, needs no test (the choices cover every value).
    fn case(&mut self, sel: ExprId, matching: bool, alts: &[(&ChoiceSet, Br)]) -> R<()> {
        let v = self.expr(sel)?;
        let v = match v {
            Val::Part(p) => Val::Part(self.materialise(p)),
            v => v,
        };
        let mut chain: Vec<(Option<Ref>, Br)> = Vec::new();
        for (k, (cs, b)) in alts.iter().enumerate() {
            let others = cs.choices.iter().any(|c| matches!(c, Choice::Others));
            if others || k + 1 == alts.len() {
                chain.push((None, *b));
                break;
            }
            let c = self.choices(&v, cs, matching)?;
            chain.push((Some(c), *b));
        }
        self.ref_chain(&chain)
    }

    /// Whether `v` matches one of the choices `cs`.
    fn choices(&mut self, v: &Val, cs: &ChoiceSet, matching: bool) -> R<Ref> {
        let mut any: Option<Ref> = None;
        for c in cs.choices.iter() {
            let t = match (v, c) {
                (Val::Int(i), Choice::Expr(x)) => {
                    let k = self.static_int(*x)?;
                    let k = self.int_lit(k);
                    self.int_cmp(Cmp::Eq, *i, k)
                }
                (Val::Int(i), Choice::Range(r)) => {
                    let (b, _) = self.static_range(r)?;
                    let (lo, hi) = b.low_high();
                    let (l, h) = (self.int_lit(lo), self.int_lit(hi));
                    let a = self.int_cmp(Cmp::Ge, *i, l);
                    let z = self.int_cmp(Cmp::Le, *i, h);
                    let t = self.tmp(1);
                    self.ops.push(Op::Gate {
                        g: Gate::And,
                        dst: t.r,
                        a,
                        b: z,
                        len: 1,
                    });
                    t.r
                }
                (Val::Part(p), Choice::Expr(x)) => {
                    let Some(c) = self.fold(*x) else {
                        return Err("a `case` choice that is not known during elaboration".into());
                    };
                    let t = self.db.base(self.db.exprs[*x].subtype);
                    let (codes, _) = self.codes(t, &c)?;
                    if codes.len() != p.len as usize {
                        return Err("a `case` choice of another length than the selector".into());
                    }
                    self.match_codes(*p, &codes, matching)?
                }
                _ => {
                    return Err(
                        "a `case` choice of this kind is not supported in cycle mode yet".into(),
                    );
                }
            };
            any = Some(match any {
                None => t,
                Some(a) => {
                    let o = self.tmp(1);
                    self.ops.push(Op::Gate {
                        g: Gate::Or,
                        dst: o.r,
                        a,
                        b: t,
                        len: 1,
                    });
                    o.r
                }
            });
        }
        any.ok_or_else(|| "a `case` alternative without choices".into())
    }

    /// `p` against the `std_ulogic` codes of a literal (from the left): `'-'` matches anything
    /// in a matching comparison; any other metavalue keeps the process out (a two-state value
    /// is never one, while the normal mode can hold it).
    fn match_codes(&mut self, p: Part, codes: &[u8], matching: bool) -> R<Ref> {
        let n = codes.len();
        let mut val = vec![0u64; words(n as u32) as usize];
        let mut care = vec![0u64; val.len()];
        for (k, &c) in codes.iter().enumerate() {
            let el = n - 1 - k;
            let (bit, keep) = match c {
                2 | 6 => (0, true),
                3 | 7 => (1, true),
                8 if matching => (0, false),
                _ => {
                    return Err(
                        "a comparison with a metavalue literal (`'U'`, `'X'`, `'-'`, ...), which \
                         two-state values never equal"
                            .into(),
                    );
                }
            };
            val[el / 64] |= bit << (el % 64);
            care[el / 64] |= u64::from(keep) << (el % 64);
        }
        let a = self.whole(p);
        let mut all = vec![0u64; val.len()];
        set_bits(&mut all, 0, n as u32);
        let a = if care != all {
            let m = self.tmp(n as u32);
            self.ops.push(Op::Lit {
                dst: m.r,
                val: care.into(),
            });
            let t = self.tmp(n as u32);
            self.ops.push(Op::Gate {
                g: Gate::And,
                dst: t.r,
                a,
                b: m.r,
                len: n as u32,
            });
            t.r
        } else {
            a
        };
        let l = self.tmp(n as u32);
        self.ops.push(Op::Lit {
            dst: l.r,
            val: val.into(),
        });
        let t = self.tmp(1);
        self.ops.push(Op::VecEq {
            dst: t.r,
            a,
            b: l.r,
            len: n as u32,
            ne: false,
        });
        Ok(t.r)
    }

    fn int_cmp(&mut self, op: Cmp, a: IntV, b: IntV) -> Ref {
        let t = self.tmp(1);
        self.ops.push(Op::IntCmp {
            op,
            dst: t.r,
            a: a.r,
            b: b.r,
        });
        t.r
    }

    /// `for i in range loop`, unrolled: the range known during elaboration, every iteration
    /// with `i` a constant.
    fn for_loop(&mut self, param: DeclId, range: &RangeC, body: &[StmtId]) -> R<()> {
        let mut inner = Vec::new();
        deep(self.db, body, &mut inner);
        if inner.iter().any(|&x| {
            matches!(
                self.db.stmts[x].kind,
                StmtKind::Next { .. } | StmtKind::Exit { .. } | StmtKind::Wait { .. }
            )
        }) {
            return Err(
                "a loop with `next`, `exit` or `wait` is not supported in cycle mode yet".into(),
            );
        }
        let (b, _) = self.static_range(range)?;
        let n = b.len();
        let size = (inner.len() as u64 + 1).saturating_mul(n);
        if size + u64::from(self.unrolled) > u64::from(UNROLL_MAX) {
            return Err(format!(
                "a loop of {n} iterations, above what cycle mode unrolls ({UNROLL_MAX} statements)"
            ));
        }
        self.unrolled += size as u32;
        for k in 0..n as i64 {
            let i = match b.dir {
                Direction::To => b.left + k,
                Direction::Downto => b.left - k,
            };
            self.consts.insert(param, i);
            self.stmts(body)?;
        }
        self.consts.remove(&param);
        Ok(())
    }

    /// A range known during elaboration (`a to b`, `x'range`), and its type.
    fn static_range(&mut self, r: &RangeC) -> R<(Bounds, Option<TypeId>)> {
        match r {
            RangeC::Explicit { left, dir, right } => Ok((
                Bounds {
                    left: self.static_int(*left)?,
                    dir: *dir,
                    right: self.static_int(*right)?,
                },
                Some(self.db.base(self.db.exprs[*left].subtype)),
            )),
            RangeC::Attr {
                prefix: AttrPrefix::Expr(x),
                reverse,
                dim: 1,
            } => {
                let b = self.name_bounds(*x)?;
                Ok((if *reverse { reversed(b) } else { b }, None))
            }
            RangeC::Attr {
                prefix: AttrPrefix::Subtype(st),
                reverse,
                dim: 1,
            } => {
                let b = self.subtype_bounds(*st)?;
                Ok((if *reverse { reversed(b) } else { b }, None))
            }
            _ => Err("a range that is not known during elaboration".into()),
        }
    }

    /// The bounds of a scalar subtype, or the index range of a one-dimensional array subtype.
    fn subtype_bounds(&mut self, st: SubtypeId) -> R<Bounds> {
        let db = self.db;
        let r = match &db.subtypes[st].constraint {
            Constraint::Range(r) => r.clone(),
            Constraint::Array { dims: Some(d), .. } if d.len() == 1 => d[0].clone(),
            _ => {
                return match db.static_bounds(st) {
                    Some((l, dir, r)) => Ok(Bounds {
                        left: l.as_int().ok_or("a range that is not discrete")?,
                        dir,
                        right: r.as_int().ok_or("a range that is not discrete")?,
                    }),
                    None => Err("a subtype whose range is not known during elaboration".into()),
                };
            }
        };
        Ok(self.static_range(&r)?.0)
    }

    /// The index range of the array object or value `x` names, without reading it.
    fn name_bounds(&mut self, x: ExprId) -> R<Bounds> {
        let (n, l) = (self.ops.len(), self.locs);
        let v = self.name(x, true);
        self.ops.truncate(n);
        self.locs = self.locs.max(l);
        match v? {
            Val::Part(Part { dims, at: None, .. }) if dims.n > 0 => Ok(dims.b[0]),
            _ => Err("an attribute of something that is not an array with a known range".into()),
        }
    }

    fn cond(&mut self, c: ExprId) -> R<Ref> {
        let c = self.expr(c)?;
        let c = self.part(c)?;
        if c.len != 1 {
            return Err("a condition that is not a boolean".into());
        }
        Ok(self.whole(c))
    }

    fn signal_rooted(&self, v: &Val) -> bool {
        match v {
            Val::Part(p) => matches!(p.r, Ref::Sig(_)),
            Val::Int(i) => matches!(i.r, Ref::Sig(_)),
            Val::Rec(fs) => fs.first().is_some_and(|f| self.signal_rooted(f)),
        }
    }

    /// The operation that made a fresh temporary writes `dst` itself.
    fn retarget(&mut self, fresh: bool, src: Ref, dst: Ref) -> bool {
        if !fresh {
            return false;
        }
        match self.ops.last_mut().and_then(|op| op.dst_mut()) {
            Some(d) if *d == src => {
                *d = dst;
                true
            }
            _ => false,
        }
    }

    /// A signal target's elements for the must-assign test: certainly assigned, or only maybe
    /// (an element chosen at run time).
    fn mark(&mut self, r: Ref, lo: u32, len: u32, sure: bool) {
        if let Ref::Sig(k) = r {
            self.may.add(k, lo, len);
            if sure {
                self.must.add(k, lo, len);
            }
        }
    }

    /// `dst := src` (or `<=`): leaf by leaf for records.
    fn assign(&mut self, dst: &Val, src: Val) -> R<()> {
        match (dst, src) {
            (Val::Rec(_), src @ Val::Rec(_)) => {
                let mut pairs = Vec::new();
                flatten(dst, src, &mut pairs)?;
                // Leaves at the same places of two blocks (the packs of two signals of one
                // record type) are copied together: one blend per pair of blocks.
                let mut groups: Vec<(Ref, Ref, Vec<u64>, u32)> = Vec::new();
                self.zero_covered(&pairs);
                for (d, s) in pairs {
                    let (Val::Part(d), Val::Part(s)) = (&d, &s) else {
                        self.assign(&d, s)?;
                        continue;
                    };
                    if s.fresh
                        || d.lo != s.lo
                        || d.len != s.len
                        || d.full != s.full
                        || d.at.is_some()
                        || s.at.is_some()
                    {
                        self.assign(&Val::Part(*d), Val::Part(*s))?;
                        continue;
                    }
                    if let Some(l) = d.leaf {
                        self.writes.insert(l);
                    }
                    self.mark(d.r, d.lo, d.len, true);
                    let g = match groups.iter().position(|g| (g.0, g.1) == (d.r, s.r)) {
                        Some(g) => g,
                        None => {
                            groups.push((d.r, s.r, vec![0; words(d.full) as usize], d.full));
                            groups.len() - 1
                        }
                    };
                    set_bits(&mut groups[g].2, d.lo, d.len);
                }
                for (dst, src, m, full) in groups {
                    if dst == src {
                        continue;
                    }
                    let mut all = vec![0; words(full) as usize];
                    set_bits(&mut all, 0, full);
                    self.ops.push(if m == all {
                        Op::Copy {
                            dst,
                            src,
                            len: full,
                        }
                    } else {
                        Op::Blend {
                            dst,
                            src,
                            mask: m.into(),
                        }
                    });
                }
                Ok(())
            }
            (Val::Part(d), Val::Part(s)) if d.len == s.len => {
                if let Some(l) = d.leaf {
                    self.writes.insert(l);
                }
                if let Some((idx, b, stride)) = d.at {
                    // ponytail: a big array signal written at a run-time index is a RAM, whose
                    // next-state copy would cost the whole array per edge; RAMs are the next step.
                    if matches!(d.r, Ref::Sig(_)) && d.full > 4096 {
                        return Err(
                            "an array signal of more than 4096 elements written at an index \
                             chosen at run time (a RAM) is not supported in cycle mode yet"
                                .into(),
                        );
                    }
                    self.mark(d.r, d.lo, stride * b.len() as u32, false);
                    let s = self.whole(s);
                    let site = self.index_site(d, b);
                    self.ops.push(Op::IndexWrite {
                        dst: d.r,
                        src: s,
                        lo: d.lo,
                        n: b.len() as u32,
                        stride,
                        w: d.len,
                        idx,
                        left: b.left,
                        down: b.dir == Direction::Downto,
                        site,
                    });
                    return Ok(());
                }
                self.mark(d.r, d.lo, d.len, true);
                let s = self.materialise(s);
                let whole = d.lo == 0 && d.len == d.full;
                let s_whole = s.lo == 0 && s.len == s.full;
                if whole && s_whole && self.retarget(s.fresh, s.r, d.r) {
                    return Ok(());
                }
                match (whole, s_whole) {
                    (true, true) if d.r == s.r => {}
                    (true, true) => self.ops.push(Op::Copy {
                        dst: d.r,
                        src: s.r,
                        len: d.len,
                    }),
                    (true, false) => self.ops.push(Op::Extract {
                        dst: d.r,
                        src: s.r,
                        lo: s.lo,
                        len: s.len,
                        full: s.full,
                    }),
                    (false, _) => {
                        let s = self.whole(s);
                        self.ops.push(Op::Insert {
                            dst: d.r,
                            src: s,
                            lo: d.lo,
                            len: d.len,
                        })
                    }
                }
                Ok(())
            }
            (Val::Part(_), Val::Part(_)) => {
                Err("an assignment of a value of another length than its target".into())
            }
            (Val::Int(d), Val::Int(s)) => {
                if let Some(l) = d.leaf {
                    self.writes.insert(l);
                }
                self.mark(d.r, 0, 1, true);
                // The subtype of the target: checked unless the value cannot leave it.
                if (s.lo < d.lo || s.hi > d.hi)
                    && let Some((sub, obj)) = d.place
                {
                    let site = self.site(Check::Range {
                        ty: subtype_name(self.db, sub),
                        lo: d.lo,
                        hi: d.hi,
                        down: d.down,
                        what: obj_desc(self.db, obj),
                    });
                    self.ops.push(Op::Range {
                        a: s.r,
                        lo: d.lo,
                        hi: d.hi,
                        site,
                    });
                }
                if d.r == s.r || self.retarget(s.fresh, s.r, d.r) {
                    return Ok(());
                }
                self.ops.push(Op::Copy {
                    dst: d.r,
                    src: s.r,
                    len: 64,
                });
                Ok(())
            }
            _ => Err("an assignment whose value has another shape than its target".into()),
        }
    }

    /// Words of a target that the leaves of a record assignment insert into and cover whole are
    /// set to 0 first: code builds them field by field without clearing each field.
    fn zero_covered(&mut self, pairs: &[(Val, Val)]) {
        let mut cover: Vec<(Ref, u32, Vec<u64>)> = Vec::new();
        let mut read = BTreeSet::new();
        for (d, s) in pairs {
            if let Val::Part(s) = s {
                read.insert(s.r);
            }
            let (Val::Part(d), Val::Part(s)) = (d, s) else {
                continue;
            };
            // Only what goes through an insert (not the blends of equal places).
            if d.at.is_some()
                || (d.lo == 0 && d.len == d.full)
                || (d.lo == s.lo && d.full == s.full)
            {
                continue;
            }
            let i = match cover.iter().position(|c| c.0 == d.r) {
                Some(i) => i,
                None => {
                    cover.push((d.r, d.full, vec![0; words(d.full) as usize]));
                    cover.len() - 1
                }
            };
            set_bits(&mut cover[i].2, d.lo, d.len);
        }
        for (r, full, m) in cover {
            let mut all = vec![0; words(full) as usize];
            set_bits(&mut all, 0, full);
            if m == all && !read.contains(&r) {
                self.ops.push(Op::Lit {
                    dst: r,
                    val: vec![0; all.len()].into(),
                });
            }
        }
    }

    fn index_site(&mut self, p: &Part, b: Bounds) -> u32 {
        let what = p
            .obj
            .map_or(String::new(), |d| self.db.decls[d].name.to_string());
        self.site(Check::Index { b, what })
    }

    /// A dynamic element read into a temporary; any other part as it is.
    fn materialise(&mut self, p: Part) -> Part {
        let Some((idx, b, stride)) = p.at else {
            return p;
        };
        let site = self.index_site(&p, b);
        let mut t = self.tmp(p.len);
        t.dims = p.dims;
        self.ops.push(Op::IndexRead {
            dst: t.r,
            src: p.r,
            lo: p.lo,
            n: b.len() as u32,
            stride,
            w: p.len,
            idx,
            left: b.left,
            down: b.dir == Direction::Downto,
            site,
        });
        t
    }

    /// The words holding exactly `p`'s elements.
    fn whole(&mut self, p: Part) -> Ref {
        let p = self.materialise(p);
        if p.lo == 0 && p.len == p.full {
            return p.r;
        }
        let t = self.loc(words(p.len));
        self.ops.push(Op::Extract {
            dst: t,
            src: p.r,
            lo: p.lo,
            len: p.len,
            full: p.full,
        });
        t
    }

    fn part(&mut self, v: Val) -> R<Part> {
        match v {
            Val::Part(p) => Ok(self.materialise(p)),
            Val::Int(_) => Err("an integer where a logic value is wanted".into()),
            Val::Rec(_) => Err("a record value as an operand".into()),
        }
    }

    fn int(&mut self, v: Val) -> R<IntV> {
        match v {
            Val::Int(i) => Ok(i),
            _ => Err("a value where an integer is wanted".into()),
        }
    }

    /// A name: a signal, a variable of the process, a constant, or a field, element or slice of
    /// one. `target`: the name of an assignment target (its signals are not reads).
    fn name(&mut self, x: ExprId, target: bool) -> R<Val> {
        let db = self.db;
        match &db.exprs[x].kind {
            ExprKind::Obj(d) => self.object(x, *d, target),
            ExprKind::Field { prefix, elem } => match self.name(*prefix, target)? {
                Val::Rec(mut fs) if (*elem as usize) < fs.len() => {
                    Ok(fs.swap_remove(*elem as usize))
                }
                _ => Err("a field of something that is not a record".into()),
            },
            ExprKind::Index { prefix, indices } => {
                let [i] = &indices[..] else {
                    return Err("an index of a multi-dimensional array".into());
                };
                let mut p = self.array_part(*prefix, target)?;
                let b = p
                    .dims
                    .first()
                    .ok_or("an index of a value with no index range")?;
                let ew = p.dims.elem_len();
                match self.static_int(*i) {
                    Ok(i) => {
                        let el = elem_of(b, i).ok_or("an index outside its array's range")?;
                        p.lo += el * ew;
                    }
                    Err(_) => {
                        let i = self.expr(*i)?;
                        let i = self.int(i)?;
                        p.at = Some((i.r, b, ew));
                    }
                }
                p.len = ew;
                p.dims = p.dims.rest();
                Ok(Val::Part(p))
            }
            ExprKind::Slice { prefix, range } => {
                let Ok((sb, _)) = self.static_range(range) else {
                    return self.dynamic_slice(*prefix, range, target);
                };
                let mut p = self.array_part(*prefix, target)?;
                let b = p
                    .dims
                    .first()
                    .ok_or("a slice of a value with no index range")?;
                let ew = p.dims.elem_len();
                if sb.dir != b.dir {
                    return Err("a slice in the other direction than its array".into());
                }
                let n = sb.len() as u32;
                if n == 0 {
                    return Err("a null slice".into());
                }
                // The slice's rightmost element is its element 0.
                let lo = elem_of(b, sb.right).ok_or("a slice outside its array's range")?;
                elem_of(b, sb.left).ok_or("a slice outside its array's range")?;
                p.lo += lo * ew;
                p.len = n * ew;
                p.dims = p.dims.with_first(sb);
                Ok(Val::Part(p))
            }
            _ => Err("a name that is not supported in cycle mode yet".into()),
        }
    }

    /// A slice with bounds chosen at run time and a length known during elaboration (`a(i + 7
    /// downto i)`): the length from the bounds with every value elaboration does not know taken
    /// as 0, 1 and 7, checked at run time.
    fn dynamic_slice(&mut self, prefix: ExprId, range: &RangeC, target: bool) -> R<Val> {
        let unknown = || {
            "a slice whose length is not known during elaboration is not supported in cycle mode \
             yet"
            .to_string()
        };
        let RangeC::Explicit { left, dir, right } = range else {
            return Err(unknown());
        };
        let len = |lx: &Self, v: i64| -> Option<i64> {
            let (l, r) = (lx.probe(*left, v)?, lx.probe(*right, v)?);
            Some(if *dir == Direction::Downto {
                l - r
            } else {
                r - l
            })
        };
        let d = len(self, 0).ok_or_else(unknown)?;
        if d < 0 || len(self, 1) != Some(d) || len(self, 7) != Some(d) {
            return Err(unknown());
        }
        let mut p = self.array_part(prefix, target)?;
        let b = p
            .dims
            .first()
            .ok_or("a slice of a value with no index range")?;
        if *dir != b.dir {
            return Err("a slice in the other direction than its array".into());
        }
        let ew = p.dims.elem_len();
        let (l, r) = (self.expr(*left)?, self.expr(*right)?);
        let (l, r) = (self.int(l)?, self.int(r)?);
        // The length the bounds give at run time is the one assumed.
        let (a, z) = if *dir == Direction::Downto {
            (l, r)
        } else {
            (r, l)
        };
        let diff = self.tmp_int(i64::MIN, i64::MAX);
        let site = self.site(Check::Arith {
            op: Arith::Sub,
            ty: "INTEGER".into(),
        });
        self.ops.push(Op::Int {
            op: Arith::Sub,
            dst: diff.r,
            a: a.r,
            b: z.r,
            lo: i64::from(i32::MIN),
            hi: i64::from(i32::MAX),
            site,
        });
        let site = self.site(Check::Range {
            ty: "INTEGER".into(),
            lo: d,
            hi: d,
            down: false,
            what: "the length of a slice chosen at run time".into(),
        });
        self.ops.push(Op::Range {
            a: diff.r,
            lo: d,
            hi: d,
            site,
        });
        p.at = Some((r.r, b, ew));
        p.len = (d as u32 + 1) * ew;
        // Its index range is the bounds' at run time: no index or slice of it is taken.
        p.dims = Dims::NONE;
        Ok(Val::Part(p))
    }

    /// The integer value of `x` with every integer subexpression elaboration does not know taken
    /// as `v` (the length of a slice chosen at run time).
    fn probe(&self, x: ExprId, v: i64) -> Option<i64> {
        if let Some(c) = self.fold(x) {
            return c.as_int();
        }
        let db = self.db;
        if !matches!(
            db.types[db.base(db.exprs[x].subtype)].kind,
            TypeKind::Integer
        ) {
            return None;
        }
        match &db.exprs[x].kind {
            ExprKind::Call { callee, .. }
                if matches!(
                    db.decls[*callee].kind.subprogram().map(|s| &s.body),
                    Some(yavs_sem::decl::Body::Implicit(_))
                ) =>
            {
                db.fold_with(x, &mut |c| self.probe(c, v).map(ConstValue::Int))
                    .ok()
                    .flatten()?
                    .as_int()
            }
            ExprKind::ImplicitConv { operand } => self.probe(*operand, v),
            _ => Some(v),
        }
    }

    fn array_part(&mut self, prefix: ExprId, target: bool) -> R<Part> {
        match self.name(prefix, target)? {
            Val::Part(p) if p.at.is_none() => Ok(p),
            // A part of an element chosen at run time, read: the element first.
            Val::Part(p) if !target => {
                self.note_reads(&Val::Part(p));
                Ok(self.materialise(p))
            }
            Val::Part(_) => Err("an index or slice of an element chosen at run time".into()),
            _ => Err("an index or slice of a record or an integer".into()),
        }
    }

    fn object(&mut self, x: ExprId, d: DeclId, target: bool) -> R<Val> {
        let (db, e) = (self.db, self.e);
        let decl = &db.decls[d];
        let yavs_sem::decl::DeclKind::Object(o) = &decl.kind else {
            return Err(format!("the name `{}`, which is not an object", decl.name));
        };
        use yavs_sem::decl::ObjClass;
        match o.class {
            ObjClass::Signal => {
                let Some(obj) = e.signal(self.p.scope, d) else {
                    return Err(format!(
                        "`{}`, which is not a signal of the design",
                        decl.name
                    ));
                };
                let so = e.nets.obj(obj);
                let mut next = so.leaves.0.0;
                let v = self.tree(&so.shape, d, &mut |_| {
                    next += 1;
                    Ref::Loc(next - 1)
                })?;
                if next != so.leaves.1.0 {
                    return Err(format!(
                        "the signal `{}`, whose leaves are not one per field",
                        decl.name
                    ));
                }
                self.place_sig(v, so)
            }
            ObjClass::Variable => {
                if let Some(v) = self.vars.get(&d) {
                    return Ok(v.clone());
                }
                let StmtKind::Process { scope, .. } = &db.stmts[self.p.stmt].kind else {
                    return Err("a variable outside a process".into());
                };
                if decl.scope != *scope {
                    return Err(format!("`{}`, a variable of another region", decl.name));
                }
                let key = (Some(self.p.scope), d);
                let up = (e.hier.scope(self.p.scope).parent, d);
                let Some(shape) = e.var_shapes.get(&key).or_else(|| e.var_shapes.get(&up)) else {
                    return Err(format!(
                        "the variable `{}`, whose shape elaboration did not compute",
                        decl.name
                    ));
                };
                let v = self.tree(shape, d, &mut |_| Ref::Loc(u32::MAX))?;
                let v = self.place_vars(v, o.default.is_some());
                self.vars.insert(d, v.clone());
                // The initial value, once at initialisation (a combinational process assigns
                // before it reads, so it never sees it).
                if let Some(init) = o.default {
                    let outer = std::mem::take(&mut self.ops);
                    let r = self.expr_h(init, Some(&v)).and_then(|s| self.assign(&v, s));
                    let made = std::mem::replace(&mut self.ops, outer);
                    r?;
                    self.init_ops.extend(made);
                }
                Ok(v)
            }
            ObjClass::Constant if target => Err("an assignment to a constant".into()),
            ObjClass::Constant => {
                if let Some(&i) = self.consts.get(&d) {
                    let mut v = self.int_lit(i);
                    if let Some(b) = int_bounds(db, o.subtype) {
                        (v.lo, v.hi) = b;
                    }
                    return Ok(Val::Int(v));
                }
                match self.fold(x) {
                    Some(c) => self.literal(db.base(o.subtype), &c),
                    None => Err(format!(
                        "the constant `{}`, whose value elaboration did not compute",
                        decl.name
                    )),
                }
            }
            _ => Err(format!(
                "`{}`, an object of a class cycle mode does not take",
                decl.name
            )),
        }
    }

    /// Give a variable's leaves their slots: a logic leaf starts at 0 (its `'U'`, counted, unless
    /// it has an initial value), an integer at its `'left`.
    fn place_vars(&mut self, v: Val, has_init: bool) -> Val {
        match v {
            Val::Part(mut p) => {
                p.r = self.loc(words(p.full));
                if !has_init {
                    self.uninit += u64::from(p.full);
                }
                if let Ref::Loc(w) = p.r {
                    self.var_words.push((w, words(p.full)));
                }
                Val::Part(p)
            }
            Val::Int(mut i) => {
                i.r = self.loc(1);
                if let Ref::Loc(w) = i.r {
                    self.var_words.push((w, 1));
                }
                let left = if i.down { i.hi } else { i.lo };
                if let Ref::Loc(w) = i.r
                    && left != 0
                    && !has_init
                {
                    self.init.push((w, left as u64));
                }
                Val::Int(i)
            }
            Val::Rec(fs) => Val::Rec(
                fs.into_iter()
                    .map(|f| self.place_vars(f, has_init))
                    .collect(),
            ),
        }
    }

    /// A signal's leaves (numbered `Loc(leaf)` by `tree`) at their kernel signal and offset; a
    /// port collapsed into part of its actual's leaves at that part.
    fn place_sig(&self, v: Val, so: &yavs_elab::SigObj) -> R<Val> {
        let nets = &self.e.nets;
        let leaf_of = |r: Ref| match r {
            Ref::Loc(l) => Ok(LeafId(l)),
            _ => Err("a signal leaf placed twice".to_string()),
        };
        match v {
            Val::Part(mut p) => {
                let l = leaf_of(p.r)?;
                let leaf = nets.leaf(l);
                let part = so.part((l.0 - so.leaves.0.0) as usize);
                let (off, len) = part.map_or((0, leaf.ty.len), |r| (r.offset, r.len));
                if len != p.len || leaf.ty.elem != yavs_kernel::Elem::Logic {
                    return Err(format!(
                        "the signal {}, whose kernel form differs",
                        self.e.leaf_name(l)
                    ));
                }
                if leaf.implicit.is_some() {
                    return Err("an implicit signal".into());
                }
                let at = nets.loc[l.index()];
                p.r = Ref::Sig(at.signal.0);
                p.lo = at.base + off;
                p.full = nets.signal_len(at.signal);
                p.leaf = Some(l);
                Ok(Val::Part(p))
            }
            Val::Int(mut i) => {
                let l = leaf_of(i.r)?;
                let leaf = nets.leaf(l);
                let at = nets.loc[l.index()];
                if leaf.ty.len != 1
                    || !so.parts.is_empty()
                    || !matches!(leaf.ty.elem, yavs_kernel::Elem::Bytes(1 | 4 | 8))
                    || nets.signal_len(at.signal) != 1
                    || leaf.implicit.is_some()
                {
                    return Err(format!(
                        "the signal {}, whose kernel form differs",
                        self.e.leaf_name(l)
                    ));
                }
                i.r = Ref::Sig(at.signal.0);
                i.leaf = Some(l);
                Ok(Val::Int(i))
            }
            Val::Rec(fs) => fs
                .into_iter()
                .map(|f| self.place_sig(f, so))
                .collect::<R<Vec<_>>>()
                .map(Val::Rec),
        }
    }

    /// Count the signal elements a value read (per leaf, from its element 0).
    fn note_reads(&mut self, v: &Val) {
        let nets = &self.e.nets;
        match v {
            Val::Part(p) => {
                if let Some(l) = p.leaf {
                    let base = nets.loc[l.index()].base;
                    let n = match p.at {
                        Some((_, b, stride)) => stride * b.len() as u32,
                        None => p.len,
                    };
                    self.reads.insert(l);
                    self.rparts.push((l, p.lo - base, n));
                }
            }
            Val::Int(IntV { leaf: Some(l), .. }) => {
                self.reads.insert(*l);
                self.rparts.push((*l, 0, 1));
            }
            Val::Int(_) => {}
            Val::Rec(fs) => fs.iter().for_each(|f| self.note_reads(f)),
        }
    }

    /// The leaves of `shape` of object `d`: records of logic scalars, logic vectors and arrays
    /// of logic vectors, integers and enumerations.
    fn tree(&self, shape: &Shape, d: DeclId, leaf: &mut dyn FnMut(u32) -> Ref) -> R<Val> {
        let db = self.db;
        match shape {
            Shape::Scalar { elem, .. } if *elem == yavs_kernel::Elem::Logic => {
                self.whole_type(shape)?;
                Ok(Val::Part(Part {
                    r: leaf(1),
                    lo: 0,
                    len: 1,
                    full: 1,
                    dims: Dims::NONE,
                    fresh: false,
                    leaf: None,
                    at: None,
                    obj: Some(d),
                    lit: None,
                }))
            }
            Shape::Scalar {
                ty,
                sub,
                range: Some(b),
                ..
            } if matches!(
                db.types[*ty].kind,
                TypeKind::Integer | TypeKind::Enum { .. }
            ) =>
            {
                let (lo, hi) = b.low_high();
                Ok(Val::Int(IntV {
                    r: leaf(1),
                    lo,
                    hi,
                    down: b.dir == Direction::Downto,
                    place: Some((*sub, d)),
                    fresh: false,
                    leaf: None,
                }))
            }
            Shape::Scalar { .. } => {
                Err("a value of a real or physical type is not supported in cycle mode yet".into())
            }
            Shape::Array { .. } => {
                let ds = self.logic_dims(shape)?;
                let n = u64::from(ds.elem_len()) * ds.b[0].len();
                if n == 0 || n > 1 << 20 {
                    return Err(format!(
                        "a vector of {n} elements (cycle mode takes 1 to 2**20 for now)"
                    ));
                }
                let n = n as u32;
                Ok(Val::Part(Part {
                    r: leaf(n),
                    lo: 0,
                    len: n,
                    full: n,
                    dims: ds,
                    fresh: false,
                    leaf: None,
                    at: None,
                    obj: Some(d),
                    lit: None,
                }))
            }
            Shape::Record { fields, .. } => fields
                .iter()
                .map(|(_, f)| self.tree(f, d, leaf))
                .collect::<R<Vec<_>>>()
                .map(Val::Rec),
        }
    }

    /// The index ranges of an array of logic elements (or of such arrays).
    fn logic_dims(&self, shape: &Shape) -> R<Dims> {
        match shape {
            Shape::Scalar {
                elem: yavs_kernel::Elem::Logic,
                ..
            } => {
                self.whole_type(shape)?;
                Ok(Dims::NONE)
            }
            Shape::Array { dims, elem, .. } => {
                let [b] = &dims[..] else {
                    return Err(
                        "a multi-dimensional array is not supported in cycle mode yet".into(),
                    );
                };
                Dims::wrap(*b, self.logic_dims(elem)?)
                    .ok_or_else(|| "an array nested more than four deep".to_string())
            }
            _ => Err(
                "an array whose elements are not logic values or arrays of them is not supported \
                 in cycle mode yet"
                    .into(),
            ),
        }
    }

    /// A logic scalar subtype that is its whole type: no assignment to it can fail a range
    /// check (`X01`, `UX01` and the like are left out).
    fn whole_type(&self, s: &Shape) -> R<()> {
        let Shape::Scalar { ty, range, .. } = s else {
            return Ok(());
        };
        let n = match &self.db.types[*ty].kind {
            TypeKind::Enum { lits } => lits.len() as u64,
            _ => 0,
        };
        match range {
            Some(b) if b.len() != n => Err(
                "a logic subtype with a range (`X01`, `UX01`, ...) is not supported in cycle mode \
                 yet"
                .into(),
            ),
            _ => Ok(()),
        }
    }

    /// The value of `x` if elaboration knows it: a literal, a folded expression, a constant, a
    /// generic, a generate or unrolled loop parameter, or an operation of those.
    fn fold(&self, x: ExprId) -> Option<ConstValue> {
        let db = self.db;
        if let Some(c) = db.exprs[x].value {
            return Some(db.consts[c].clone());
        }
        match &db.exprs[x].kind {
            ExprKind::Obj(d) => {
                if let Some(&i) = self.consts.get(d) {
                    return Some(ConstValue::Int(i));
                }
                match &db.decls[*d].kind {
                    yavs_sem::decl::DeclKind::Object(o)
                        if o.class == yavs_sem::decl::ObjClass::Constant =>
                    {
                        // A constant of the regions around the process, else of a package.
                        (self.e.value(self.p.scope, *d))
                            .or_else(|| {
                                (self.e.hier.scopes.iter().enumerate())
                                    .filter(|(_, sc)| {
                                        matches!(sc.kind, yavs_elab::ScopeKind::Package { .. })
                                    })
                                    .find_map(|(k, _)| self.e.model_vals.get(k)?.get(d))
                            })
                            .cloned()
                    }
                    _ => None,
                }
            }
            ExprKind::ImplicitConv { operand } => self.fold(*operand),
            _ => db.fold_with(x, &mut |c| self.fold(c)).ok().flatten(),
        }
    }

    fn static_int(&mut self, x: ExprId) -> R<i64> {
        if let Some(v) = self.fold(x).and_then(|c| c.as_int()) {
            return Ok(v);
        }
        // `a'length` and its friends of an object whose range elaboration fixed.
        if let ExprKind::Attr {
            attr,
            prefix: AttrPrefix::Expr(p),
            args,
        } = &self.db.exprs[x].kind
            && args.is_empty()
            && let Ok(b) = self.name_bounds(*p)
        {
            let (lo, hi) = b.low_high();
            match attr {
                PredefAttr::Length => return Ok(b.len() as i64),
                PredefAttr::Left => return Ok(b.left),
                PredefAttr::Right => return Ok(b.right),
                PredefAttr::Low => return Ok(lo),
                PredefAttr::High => return Ok(hi),
                _ => {}
            }
        }
        Err(format!(
            "a value that is not known during elaboration (`{}`)",
            self.text(x)
        ))
    }

    /// The source text of `x`, short, on one line (reasons in the region report).
    fn text(&self, x: ExprId) -> String {
        let sp = self.db.exprs[x].span;
        let f = self.db.sources.get(sp.file);
        let (a, b) = (f.text_offset(sp.start), f.text_offset(sp.end));
        let t: String = f
            .text()
            .get(a..b)
            .unwrap_or("")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        match t.char_indices().nth(40) {
            Some((k, _)) => format!("{}...", &t[..k]),
            None => t,
        }
    }

    fn expr(&mut self, x: ExprId) -> R<Val> {
        self.expr_h(x, None)
    }

    /// Expression `x`; `hint`: the place it is assigned to, whose shape an aggregate takes.
    fn expr_h(&mut self, x: ExprId, hint: Option<&Val>) -> R<Val> {
        let db = self.db;
        let ex = &db.exprs[x];
        if let Some(c) = self.fold(x) {
            return self.literal(db.base(ex.subtype), &c);
        }
        match &ex.kind {
            ExprKind::Obj(_)
            | ExprKind::Field { .. }
            | ExprKind::Index { .. }
            | ExprKind::Slice { .. } => {
                let v = self.name(x, false)?;
                self.note_reads(&v);
                match v {
                    Val::Part(p) => Ok(Val::Part(self.materialise(p))),
                    v => Ok(v),
                }
            }
            ExprKind::Attr {
                attr:
                    PredefAttr::Length
                    | PredefAttr::Left
                    | PredefAttr::Right
                    | PredefAttr::Low
                    | PredefAttr::High,
                ..
            } => {
                let v = self.static_int(x)?;
                Ok(Val::Int(self.int_lit(v)))
            }
            ExprKind::Attr { .. } => {
                Err("an attribute that is not supported in cycle mode yet".into())
            }
            ExprKind::Aggregate { assocs, bounds } => self.aggregate(x, assocs, bounds, hint),
            ExprKind::Qualified { to, operand } | ExprKind::Conv { to, operand } => {
                self.convert(x, *to, *operand, hint)
            }
            ExprKind::ImplicitConv { operand } => self.expr_h(*operand, hint),
            ExprKind::Call {
                callee,
                args,
                method_of: None,
                ..
            } => self.call(x, *callee, args),
            _ => Err("an expression that is not supported in cycle mode yet".into()),
        }
    }

    /// A qualified expression or a type conversion: logic arrays keep their elements (LRM
    /// 9.3.6), integers and enumerations are checked against a narrower subtype.
    fn convert(&mut self, x: ExprId, to: SubtypeId, operand: ExprId, hint: Option<&Val>) -> R<Val> {
        let db = self.db;
        let open = matches!(
            db.subtypes[to].constraint,
            Constraint::None | Constraint::Array { dims: None, .. }
        );
        match self.expr_h(operand, hint)? {
            Val::Part(p) => {
                let from = db.exprs[operand].subtype;
                let qualified = matches!(db.exprs[x].kind, ExprKind::Qualified { .. });
                if !(qualified || self.logic_array(to) && self.logic_array(from)) {
                    return Err("a type conversion that is not between logic vector types".into());
                }
                if !open
                    && let Ok(b) = self.subtype_bounds(to)
                    && b.len() != u64::from(p.len)
                {
                    return Err("a conversion to a subtype of another length".into());
                }
                Ok(Val::Part(p))
            }
            Val::Int(i) => {
                let Some((lo, hi)) = int_bounds(db, to) else {
                    return Err("a conversion to a type that is not an integer type".into());
                };
                if i.lo >= lo && i.hi <= hi {
                    return Ok(Val::Int(i));
                }
                let site = self.site(Check::Range {
                    ty: subtype_name(db, to),
                    lo,
                    hi,
                    down: false,
                    what: "conversion".into(),
                });
                self.ops.push(Op::Range {
                    a: i.r,
                    lo,
                    hi,
                    site,
                });
                Ok(Val::Int(IntV {
                    lo: i.lo.max(lo),
                    hi: i.hi.min(hi),
                    place: None,
                    ..i
                }))
            }
            Val::Rec(_) => Err("a conversion of a record".into()),
        }
    }

    /// An array aggregate of logic elements in the shape of `hint` (or of its own subtype), or a
    /// record aggregate.
    fn aggregate(
        &mut self,
        x: ExprId,
        assocs: &[yavs_sem::hir::AggAssoc],
        bounds: &AggBounds,
        hint: Option<&Val>,
    ) -> R<Val> {
        let db = self.db;
        if let AggBounds::Record = bounds {
            let TypeKind::Record { elems } = &db.types[db.base(db.exprs[x].subtype)].kind else {
                return Err("a record aggregate of something that is not a record".into());
            };
            let fields = match hint {
                Some(Val::Rec(fs)) if fs.len() == elems.len() => Some(fs.clone()),
                _ => None,
            };
            let mut out: Vec<Option<Val>> = vec![None; elems.len()];
            for (k, a) in assocs.iter().enumerate() {
                let at: Vec<usize> = if a.choices.is_empty() {
                    vec![k]
                } else if a.choices.iter().any(|c| matches!(c, Choice::Others)) {
                    (0..elems.len()).filter(|&f| out[f].is_none()).collect()
                } else {
                    (a.choices.iter())
                        .map(|c| match c {
                            Choice::Element(f) => Ok(*f as usize),
                            _ => Err("a record aggregate choice of this kind".to_string()),
                        })
                        .collect::<R<_>>()?
                };
                for f in at {
                    let h = fields.as_ref().map(|fs| fs[f].clone());
                    out[f] = Some(self.expr_h(a.value, h.as_ref())?);
                }
            }
            return out
                .into_iter()
                .map(|v| v.ok_or_else(|| "a record aggregate that misses a field".to_string()))
                .collect::<R<Vec<_>>>()
                .map(Val::Rec);
        }
        // The shape: the place's, or a fully constrained subtype's.
        let (n, dims) = match hint {
            Some(Val::Part(p)) if p.dims.n > 0 => (p.len, p.dims),
            _ => {
                let st = db.exprs[x].subtype;
                let b = self.subtype_bounds(st)?;
                let TypeKind::Array { elem, .. } = &db.types[db.base(st)].kind else {
                    return Err("an array aggregate of something that is not an array".into());
                };
                if yavs_elab::elem_of(db, db.base(*elem)) != yavs_kernel::Elem::Logic {
                    return Err(
                        "an aggregate of vectors whose shape is not known here is not supported \
                         in cycle mode yet"
                            .into(),
                    );
                }
                (b.len() as u32, Dims::wrap(b, Dims::NONE).expect("one deep"))
            }
        };
        let (b, ew) = (dims.b[0], dims.elem_len());
        if b.len() as u32 * ew != n || n == 0 {
            return Err("an aggregate of a null or unexpected shape".into());
        }
        let elem_hint = Val::Part(Part {
            r: Ref::Loc(0),
            lo: 0,
            len: ew,
            full: ew,
            dims: dims.rest(),
            fresh: false,
            leaf: None,
            at: None,
            obj: None,
            lit: None,
        });
        let count = b.len() as usize;
        let mut done = vec![false; count];
        // Element positions from the left, with their values.
        let mut put: Vec<(Vec<usize>, Part)> = Vec::new();
        for (k, a) in assocs.iter().enumerate() {
            if a.slice_piece {
                return Err(
                    "an aggregate with slice pieces is not supported in cycle mode yet".into(),
                );
            }
            let mut at: Vec<usize> = Vec::new();
            if a.choices.is_empty() {
                at.push(k);
            }
            for c in a.choices.iter() {
                match c {
                    Choice::Others => at.extend((0..count).filter(|&q| !done[q])),
                    Choice::Expr(i) => {
                        let i = self.static_int(*i)?;
                        let p = b.pos(i).ok_or("an aggregate choice outside the range")?;
                        at.push(p as usize);
                    }
                    Choice::Range(r) => {
                        let (rb, _) = self.static_range(r)?;
                        let (lo, hi) = rb.low_high();
                        for i in lo..=hi {
                            at.push(
                                b.pos(i).ok_or("an aggregate choice outside the range")? as usize
                            );
                        }
                    }
                    Choice::Element(_) => {
                        return Err("a record choice in an array aggregate".into());
                    }
                }
            }
            for &q in &at {
                if q >= count {
                    return Err("an aggregate with too many elements".into());
                }
                done[q] = true;
            }
            let v = self.expr_h(a.value, Some(&elem_hint))?;
            let v = self.part(v)?;
            if v.len != ew {
                return Err("an aggregate element of another length".into());
            }
            put.push((at, v));
        }
        if done.iter().any(|d| !d) {
            return Err("an aggregate that misses an element".into());
        }
        let mut t = self.tmp(n);
        t.dims = dims;
        // One value for every element of an array of scalars: spread it.
        if let [(at, v)] = &put[..]
            && ew == 1
            && at.len() == count
        {
            let a = self.whole(*v);
            self.ops.push(Op::Splat {
                dst: t.r,
                a,
                len: n,
            });
            return Ok(Val::Part(t));
        }
        self.ops.push(Op::Lit {
            dst: t.r,
            val: vec![0; words(n) as usize].into(),
        });
        for (at, v) in put {
            let s = self.whole(v);
            for q in at {
                // Position `q` from the left is element `count - 1 - q`.
                self.ops.push(Op::Insert {
                    dst: t.r,
                    src: s,
                    lo: (count - 1 - q) as u32 * ew,
                    len: ew,
                });
            }
        }
        Ok(Val::Part(t))
    }

    fn logic_array(&self, st: SubtypeId) -> bool {
        let db = self.db;
        match &db.types[db.base(st)].kind {
            TypeKind::Array { indexes, elem } => {
                indexes.len() == 1
                    && yavs_elab::elem_of(db, db.base(*elem)) == yavs_kernel::Elem::Logic
            }
            _ => false,
        }
    }

    /// The `std_ulogic` code of enumeration position `pos` of logic type `t` (BIT and BOOLEAN as
    /// `'0'`/`'1'`).
    fn code(&self, t: TypeId, pos: u32) -> R<u8> {
        let wk = &self.db.well_known;
        if Some(t) == wk.bit || Some(t) == wk.boolean {
            return Ok(pos as u8 + 2);
        }
        if yavs_elab::elem_of(self.db, t) != yavs_kernel::Elem::Logic || pos >= 9 {
            return Err("a literal of a type that is not a logic type".into());
        }
        Ok(pos as u8)
    }

    /// The `std_ulogic` codes of a logic value of type `t`, from the left, with its index ranges.
    fn codes(&self, t: TypeId, c: &ConstValue) -> R<(Vec<u8>, Dims)> {
        let db = self.db;
        match (c, &db.types[t].kind) {
            (ConstValue::Enum(p), _) => Ok((vec![self.code(t, *p)?], Dims::NONE)),
            (ConstValue::Array(a), TypeKind::Array { elem, .. }) => {
                let [(l, d, r)] = a.bounds[..] else {
                    return Err("a multi-dimensional literal".into());
                };
                let b = Bounds {
                    left: l,
                    dir: d,
                    right: r,
                };
                let et = db.base(*elem);
                let mut out = Vec::new();
                let mut inner = Dims::NONE;
                match &a.elems {
                    Elems::Bytes(bs) => {
                        for &p in bs.iter() {
                            out.push(self.code(et, u32::from(p))?);
                        }
                    }
                    Elems::Runs(runs) => {
                        for (n, v) in runs.iter() {
                            let (cs, ib) = self.codes(et, v)?;
                            inner = ib;
                            for _ in 0..*n {
                                out.extend_from_slice(&cs);
                            }
                        }
                    }
                }
                let ds = Dims::wrap(b, inner).ok_or("a literal nested more than four deep")?;
                if out.len() as u64 != b.len() * u64::from(ds.elem_len()) {
                    return Err("a literal of an unexpected shape".into());
                }
                Ok((out, ds))
            }
            _ => Err("a literal that is not a logic value".into()),
        }
    }

    /// A value known during elaboration, of type `t`.
    fn literal(&mut self, t: TypeId, c: &ConstValue) -> R<Val> {
        let db = self.db;
        match (c, &db.types[t].kind) {
            (ConstValue::Int(v), TypeKind::Integer | TypeKind::UniversalInteger) => {
                return Ok(Val::Int(self.int_lit(*v)));
            }
            (ConstValue::Enum(p), TypeKind::Enum { .. })
                if yavs_elab::elem_of(db, t) != yavs_kernel::Elem::Logic =>
            {
                return Ok(Val::Int(self.int_lit(i64::from(*p))));
            }
            (ConstValue::Record(fs), TypeKind::Record { elems }) if fs.len() == elems.len() => {
                return (fs.iter().zip(elems.iter()))
                    .map(|(f, el)| self.literal(db.base(el.subtype), f))
                    .collect::<R<Vec<_>>>()
                    .map(Val::Rec);
            }
            _ => {}
        }
        let (codes, dims) = self.codes(t, c)?;
        let n = codes.len() as u32;
        if n == 0 {
            return Err("a null literal".into());
        }
        let mut val = vec![0u64; words(n) as usize];
        let mut meta = false;
        for (p, &code) in codes.iter().enumerate() {
            let bit = match code {
                2 | 6 => 0,
                3 | 7 => 1,
                4 => return Err("a `'Z'` literal: two-state values have no high impedance".into()),
                _ => {
                    meta = true;
                    0
                }
            };
            // Row-major element `p` from the left is element `n - 1 - p`.
            let el = n as usize - 1 - p;
            val[el / 64] |= bit << (el % 64);
        }
        if meta {
            self.metas += 1;
        }
        let mut t = self.tmp(n);
        self.ops.push(Op::Lit {
            dst: t.r,
            val: val.into(),
        });
        t.dims = dims;
        if meta {
            self.lits.push(codes);
            t.lit = Some(self.lits.len() as u32 - 1);
        }
        Ok(Val::Part(t))
    }

    fn arg(&mut self, a: &yavs_sem::hir::Arg) -> R<Val> {
        let Actual::Expr(x) = a.actual else {
            return Err("a call with a default or converted actual".into());
        };
        self.expr(x)
    }

    /// The range of the result type of expression `x` (an integer operation), and its name.
    fn result_range(&self, x: ExprId) -> R<(i64, i64, String)> {
        let db = self.db;
        let t = db.base(db.exprs[x].subtype);
        let b = db.types[t].base_subtype;
        match db.static_bounds(b) {
            Some((ConstValue::Int(l), dir, ConstValue::Int(r))) => {
                let (lo, hi) = if dir == Direction::Downto {
                    (r, l)
                } else {
                    (l, r)
                };
                Ok((lo, hi, subtype_name(db, b)))
            }
            _ => Err("an integer type whose range is not static".into()),
        }
    }

    /// `p` extended (or cut) to `to` elements.
    fn ext(&mut self, p: Part, to: u32, signed: bool) -> Part {
        if p.len == to {
            return p;
        }
        let s = self.whole(p);
        let t = self.tmp(to);
        self.ops.push(Op::Ext {
            dst: t.r,
            src: s,
            from: p.len,
            to,
            signed,
        });
        t
    }

    /// The integer `i` as a vector of `len` elements (`to_unsigned`, `to_signed`): its low bits.
    fn int_vec(&mut self, i: IntV, len: u32) -> Part {
        let t = self.tmp(len);
        self.ops.push(Op::FromInt {
            dst: t.r,
            a: i.r,
            len,
        });
        t
    }

    /// `i` passed to a `NATURAL` parameter: checked unless it cannot be negative.
    fn natural(&mut self, i: IntV) {
        if i.lo >= 0 {
            return;
        }
        let hi = i64::from(i32::MAX);
        let site = self.site(Check::Range {
            ty: "NATURAL".into(),
            lo: 0,
            hi,
            down: false,
            what: "a parameter".into(),
        });
        self.ops.push(Op::Range {
            a: i.r,
            lo: 0,
            hi,
            site,
        });
    }

    /// The operands of a `numeric_std` operation of form `form` as vectors of one length: the
    /// longer vector's for two vectors, the vector's for a vector and an integer.
    fn num_operands(
        &mut self,
        args: &[yavs_sem::hir::Arg],
        form: yavs_ir::ieee::Form,
        signed: bool,
    ) -> R<(Part, Part, u32)> {
        use yavs_ir::ieee::Form;
        let [a, b] = args else {
            return Err("a `numeric_std` operator with an unexpected number of arguments".into());
        };
        let (a, b) = (self.arg(a)?, self.arg(b)?);
        Ok(match form {
            Form::Vv => {
                let (a, b) = (self.part(a)?, self.part(b)?);
                let n = a.len.max(b.len);
                (self.ext(a, n, signed), self.ext(b, n, signed), n)
            }
            Form::Vi => {
                let a = self.part(a)?;
                let b = self.int(b)?;
                if !signed {
                    self.natural(b);
                }
                let n = a.len;
                (a, self.int_vec(b, n), n)
            }
            Form::Iv => {
                let a = self.int(a)?;
                let b = self.part(b)?;
                if !signed {
                    self.natural(a);
                }
                let n = b.len;
                (self.int_vec(a, n), b, n)
            }
        })
    }

    /// A shift of `p` by a count known during elaboration: element moves only.
    fn static_shift(&mut self, k: ShiftK, p: Part, n: i64) -> Part {
        let len = p.len;
        let (k, n) = match k {
            ShiftK::Sll | ShiftK::Srl | ShiftK::Sra if n < 0 => (
                match k {
                    ShiftK::Sll => ShiftK::Srl,
                    _ => ShiftK::Sll,
                },
                -n,
            ),
            ShiftK::Rol | ShiftK::Ror => (k, n.rem_euclid(i64::from(len))),
            _ => (k, n),
        };
        let a = self.whole(p);
        let t = self.tmp(len);
        let ext = |lx: &mut Self, lo: u32, l: u32| {
            let e = lx.loc(words(l));
            lx.ops.push(Op::Extract {
                dst: e,
                src: a,
                lo,
                len: l,
                full: len,
            });
            e
        };
        if n == 0 {
            self.ops.push(Op::Copy {
                dst: t.r,
                src: a,
                len,
            });
            return t;
        }
        if n >= i64::from(len) && matches!(k, ShiftK::Sll | ShiftK::Srl) {
            self.ops.push(Op::Lit {
                dst: t.r,
                val: vec![0; words(len) as usize].into(),
            });
            return t;
        }
        let n = n.min(i64::from(len)) as u32;
        match k {
            ShiftK::Sll => {
                let hi = ext(self, 0, len - n);
                let z = self.loc(words(n));
                self.ops.push(Op::Lit {
                    dst: z,
                    val: vec![0; words(n) as usize].into(),
                });
                self.ops.push(Op::Cat {
                    dst: t.r,
                    hi,
                    lo: z,
                    lo_len: n,
                    len,
                });
            }
            ShiftK::Srl => {
                self.ops.push(Op::Extract {
                    dst: t.r,
                    src: a,
                    lo: n,
                    len: len - n,
                    full: len,
                });
            }
            ShiftK::Sra => {
                let s = ext(self, len - 1, 1);
                let fill = self.loc(words(n));
                self.ops.push(Op::Splat {
                    dst: fill,
                    a: s,
                    len: n,
                });
                if n == len {
                    self.ops.push(Op::Copy {
                        dst: t.r,
                        src: fill,
                        len,
                    });
                } else {
                    let lo = ext(self, n, len - n);
                    self.ops.push(Op::Cat {
                        dst: t.r,
                        hi: fill,
                        lo,
                        lo_len: len - n,
                        len,
                    });
                }
            }
            ShiftK::Rol | ShiftK::Ror => {
                // Left by `n` is right by `len - n`.
                let n = if k == ShiftK::Rol { n } else { len - n };
                let hi = ext(self, 0, len - n);
                let lo = ext(self, len - n, n);
                self.ops.push(Op::Cat {
                    dst: t.r,
                    hi,
                    lo,
                    lo_len: n,
                    len,
                });
            }
        }
        t
    }

    fn call(&mut self, x: ExprId, callee: DeclId, args: &[yavs_sem::hir::Arg]) -> R<Val> {
        let db = self.db;
        let decl = &db.decls[callee];
        let Some(sp) = decl.kind.subprogram() else {
            return Err("a call of something that is not a subprogram".into());
        };
        use yavs_ir::ieee::{Cmp as C, Form, Gate as G, Op as I, Shift as S};
        let gate = |g: G| match g {
            G::And => Gate::And,
            G::Or => Gate::Or,
            G::Xor => Gate::Xor,
            G::Nand => Gate::Nand,
            G::Nor => Gate::Nor,
            G::Xnor => Gate::Xnor,
        };
        let cmp = |c: C| match c {
            C::Eq => Cmp::Eq,
            C::Ne => Cmp::Ne,
            C::Lt => Cmp::Lt,
            C::Le => Cmp::Le,
            C::Gt => Cmp::Gt,
            C::Ge => Cmp::Ge,
        };
        let shift = |s: S| match s {
            S::Sll => ShiftK::Sll,
            S::Srl => ShiftK::Srl,
            S::Sra => ShiftK::Sra,
            S::Rol => ShiftK::Rol,
            S::Ror => ShiftK::Ror,
        };
        let not_yet = || {
            Err(format!(
                "a call of `{}`, which is not supported in cycle mode yet",
                decl.name
            ))
        };
        let one = |lx: &mut Self| -> R<Val> {
            let Some(a) = args.first() else {
                return Err("a call without arguments".into());
            };
            lx.arg(a)
        };
        let two = |lx: &mut Self| -> R<(Val, Val)> {
            let [a, b] = args else {
                return Err(format!(
                    "a call of `{}` with an unexpected number of arguments",
                    decl.name
                ));
            };
            let a = lx.arg(a)?;
            // The second operand takes the shape of the first (an aggregate).
            let Actual::Expr(bx) = b.actual else {
                return Err("a call with a default or converted actual".into());
            };
            let b = lx.expr_h(bx, Some(&a))?;
            Ok((a, b))
        };
        match &sp.body {
            yavs_sem::decl::Body::Implicit(op) => {
                let ty = op.ty;
                let int = match &db.types[ty].kind {
                    TypeKind::Integer | TypeKind::UniversalInteger => true,
                    TypeKind::Enum { .. } => yavs_elab::elem_of(db, ty) != yavs_kernel::Elem::Logic,
                    _ => false,
                };
                let short = Some(ty) == db.well_known.boolean || Some(ty) == db.well_known.bit;
                let fam = op.family;
                let rel = match fam {
                    Family::Eq => Some(Cmp::Eq),
                    Family::Neq => Some(Cmp::Ne),
                    Family::Lt => Some(Cmp::Lt),
                    Family::Le => Some(Cmp::Le),
                    Family::Gt => Some(Cmp::Gt),
                    Family::Ge => Some(Cmp::Ge),
                    _ => None,
                };
                let ar = match fam {
                    Family::Add => Some(Arith::Add),
                    Family::Sub => Some(Arith::Sub),
                    Family::Mul => Some(Arith::Mul),
                    Family::Div => Some(Arith::Div),
                    Family::Mod => Some(Arith::Mod),
                    Family::Rem => Some(Arith::Rem),
                    _ => None,
                };
                let g = match fam {
                    Family::And => Some(Gate::And),
                    Family::Or => Some(Gate::Or),
                    Family::Xor => Some(Gate::Xor),
                    Family::Nand => Some(Gate::Nand),
                    Family::Nor => Some(Gate::Nor),
                    Family::Xnor => Some(Gate::Xnor),
                    _ => None,
                };
                if int {
                    if let Some(c) = rel {
                        let (a, b) = two(self)?;
                        let (a, b) = (self.int(a)?, self.int(b)?);
                        return Ok(Val::Part(part_of(self.int_cmp(c, a, b), 1)));
                    }
                    if let Some(op) = ar {
                        let (a, b) = two(self)?;
                        let (a, b) = (self.int(a)?, self.int(b)?);
                        return self.int_arith(x, op, a, b).map(Val::Int);
                    }
                    return match fam {
                        Family::Neg => {
                            let a = {
                                let v = one(self)?;
                                self.int(v)
                            }?;
                            let z = self.int_lit(0);
                            self.int_arith(x, Arith::Sub, z, a).map(Val::Int)
                        }
                        Family::Pos => one(self),
                        Family::Abs => {
                            let a = {
                                let v = one(self)?;
                                self.int(v)
                            }?;
                            let z = self.int_lit(0);
                            let n = self.int_arith(x, Arith::Sub, z, a)?;
                            let c = self.int_cmp(Cmp::Lt, a, z);
                            let t = self
                                .tmp_int(0, a.lo.unsigned_abs().max(a.hi.unsigned_abs()) as i64);
                            self.ops.push(Op::Select {
                                dst: t.r,
                                c,
                                a: n.r,
                                b: a.r,
                                len: 64,
                            });
                            Ok(Val::Int(t))
                        }
                        Family::Minimum | Family::Maximum => {
                            let (a, b) = two(self)?;
                            let (a, b) = (self.int(a)?, self.int(b)?);
                            let c = self.int_cmp(Cmp::Lt, a, b);
                            let t = self.tmp_int(a.lo.min(b.lo), a.hi.max(b.hi));
                            let (p, q) = if fam == Family::Minimum {
                                (a, b)
                            } else {
                                (b, a)
                            };
                            self.ops.push(Op::Select {
                                dst: t.r,
                                c,
                                a: p.r,
                                b: q.r,
                                len: 64,
                            });
                            Ok(Val::Int(t))
                        }
                        _ => Err(format!(
                            "the predefined `{}` on integers, which is not supported in cycle \
                             mode yet",
                            decl.name
                        )),
                    };
                }
                if let Some(g) = g {
                    if short && matches!(g, Gate::And | Gate::Or | Gate::Nand | Gate::Nor) {
                        return self.short_circuit(g, args);
                    }
                    let (a, b) = two(self)?;
                    return self.gate(g, a, b);
                }
                match fam {
                    Family::Not => {
                        let v = one(self)?;
                        self.not(v)
                    }
                    Family::Cat => {
                        let (a, b) = two(self)?;
                        let (a, b) = (self.part(a)?, self.part(b)?);
                        let (ra, rb) = (self.whole(a), self.whole(b));
                        let t = self.tmp(a.len + b.len);
                        self.ops.push(Op::Cat {
                            dst: t.r,
                            hi: ra,
                            lo: rb,
                            lo_len: b.len,
                            len: a.len + b.len,
                        });
                        Ok(Val::Part(t))
                    }
                    Family::Eq | Family::Neq | Family::MEq | Family::MNeq => {
                        let (a, b) = two(self)?;
                        let ne = matches!(fam, Family::Neq | Family::MNeq);
                        self.vec_eq(a, b, ne, matches!(fam, Family::MEq | Family::MNeq))
                    }
                    // `??` of BIT: the value itself.
                    Family::Cond => one(self),
                    _ => Err(format!(
                        "the predefined `{}` on these operands, which is not supported in cycle \
                         mode yet",
                        decl.name
                    )),
                }
            }
            _ => {
                let Some(op) = yavs_ir::ieee::native_symbol(sp.mangled.as_str())
                    .as_deref()
                    .and_then(yavs_ir::ieee::symbol_key)
                    .and_then(yavs_ir::ieee::parse)
                else {
                    return not_yet();
                };
                match op {
                    I::Gate(g) | I::VecGate(g) | I::NumGate(g) => {
                        let (a, b) = two(self)?;
                        self.gate(gate(g), a, b)
                    }
                    I::VecScalarGate { g, left } => {
                        let (a, b) = two(self)?;
                        let (v, s) = if left { (b, a) } else { (a, b) };
                        let v = self.part(v)?;
                        let s = self.part(s)?;
                        let rs = self.whole(s);
                        let t = self.tmp(v.len);
                        self.ops.push(Op::Splat {
                            dst: t.r,
                            a: rs,
                            len: v.len,
                        });
                        self.gate(gate(g), Val::Part(v), Val::Part(t))
                    }
                    I::Not | I::VecNot | I::NumNot => {
                        let v = one(self)?;
                        self.not(v)
                    }
                    I::Reduce(g) => {
                        let a = {
                            let v = one(self)?;
                            self.part(v)
                        }?;
                        let ra = self.whole(a);
                        let t = self.tmp(1);
                        self.ops.push(Op::Reduce {
                            g: gate(g),
                            dst: t.r,
                            a: ra,
                            len: a.len,
                        });
                        Ok(Val::Part(t))
                    }
                    // Identities in two-state values.
                    I::ToSulv | I::Conv(..) | I::To01 | I::ToBitVector | I::Truthy => one(self),
                    // A two-state value is never a metavalue.
                    I::IsX(_) => Ok(Val::Part(self.bool_lit(false))),
                    I::Add { sub, signed, form } => {
                        let (a, b, n) = self.num_operands(args, form, signed)?;
                        let (ra, rb) = (self.whole(a), self.whole(b));
                        let t = self.tmp(n);
                        self.ops.push(Op::Add {
                            dst: t.r,
                            a: ra,
                            b: rb,
                            len: n,
                            sub,
                        });
                        Ok(Val::Part(t))
                    }
                    I::Mul { signed, form } => {
                        // The result has the lengths of both: twice the vector's with an integer.
                        let (a, b, m) = if form == Form::Vv {
                            let (a, b) = two(self)?;
                            let (a, b) = (self.part(a)?, self.part(b)?);
                            let m = a.len + b.len;
                            (a, b, m)
                        } else {
                            let (a, b, n) = self.num_operands(args, form, signed)?;
                            (a, b, 2 * n)
                        };
                        let a = self.ext(a, m, signed);
                        let b = self.ext(b, m, signed);
                        let (ra, rb) = (self.whole(a), self.whole(b));
                        let t = self.tmp(m);
                        self.ops.push(Op::Mul {
                            dst: t.r,
                            a: ra,
                            b: rb,
                            len: m,
                        });
                        Ok(Val::Part(t))
                    }
                    I::Div { kind, signed, form } => self.num_div(args, kind, signed, form),
                    I::Compare {
                        cmp: c,
                        signed,
                        form,
                    }
                    | I::QCompare {
                        cmp: c,
                        signed,
                        form,
                    } => self.num_compare(args, cmp(c), signed, form),
                    I::MinMax {
                        max,
                        signed,
                        form: Form::Vv,
                    } => {
                        let (a, b, n) = self.num_operands(args, Form::Vv, signed)?;
                        let (ra, rb) = (self.whole(a), self.whole(b));
                        let c = self.tmp(1);
                        self.ops.push(Op::VCmp {
                            cmp: Cmp::Lt,
                            signed,
                            dst: c.r,
                            a: ra,
                            b: rb,
                            len: n,
                        });
                        let t = self.tmp(n);
                        let (p, q) = if max { (rb, ra) } else { (ra, rb) };
                        self.ops.push(Op::Select {
                            dst: t.r,
                            c: c.r,
                            a: p,
                            b: q,
                            len: n,
                        });
                        Ok(Val::Part(t))
                    }
                    I::Match | I::MatchEq { .. } => {
                        let ne = matches!(op, I::MatchEq { ne: true, .. });
                        let (a, b) = two(self)?;
                        self.vec_eq(a, b, ne, true)
                    }
                    I::ToInteger { signed } => {
                        let a = {
                            let v = one(self)?;
                            self.part(v)
                        }?;
                        if a.len > 64 {
                            return Err("`to_integer` of a vector of more than 64 elements".into());
                        }
                        let ra = self.whole(a);
                        let (lo, hi) = if signed {
                            (
                                -(1i64 << (a.len - 1).min(62)),
                                (1i64 << (a.len - 1).min(62)) - 1,
                            )
                        } else {
                            (0, (1i64 << a.len.min(62)) - 1)
                        };
                        let t = self.tmp_int(lo.max(i32::MIN.into()), hi.min(i32::MAX.into()));
                        self.ops.push(Op::ToInt {
                            dst: t.r,
                            a: ra,
                            len: a.len,
                            signed,
                        });
                        // The result is an INTEGER (A7: kept checks).
                        if lo < i32::MIN.into() || hi > i32::MAX.into() || (!signed && a.len == 64)
                        {
                            let (l, h) = if signed {
                                (i32::MIN.into(), i32::MAX.into())
                            } else {
                                (0, i32::MAX.into())
                            };
                            let site = self.site(Check::Range {
                                ty: if signed { "INTEGER" } else { "NATURAL" }.into(),
                                lo: l,
                                hi: h,
                                down: false,
                                what: "the result of `to_integer`".into(),
                            });
                            self.ops.push(Op::Range {
                                a: t.r,
                                lo: l,
                                hi: h,
                                site,
                            });
                        }
                        Ok(Val::Int(t))
                    }
                    I::ToUnsigned | I::ToSigned => {
                        let [a, n] = args else {
                            return Err(
                                "a conversion with an unexpected number of arguments".into()
                            );
                        };
                        let Actual::Expr(nx) = n.actual else {
                            return Err("a size that is not given".into());
                        };
                        let n = self.static_int(nx).map_err(|_| {
                            "a vector size that is not known during elaboration".to_string()
                        })?;
                        if n <= 0 || n > 1 << 20 {
                            return Err("a null or very large vector size".into());
                        }
                        let a = self.arg(a)?;
                        let a = self.int(a)?;
                        if op == I::ToUnsigned {
                            self.natural(a);
                        }
                        Ok(Val::Part(self.int_vec(a, n as u32)))
                    }
                    I::Resize { signed } => {
                        let [a, n] = args else {
                            return Err("`resize` with an unexpected number of arguments".into());
                        };
                        let Actual::Expr(nx) = n.actual else {
                            return Err("a size that is not given".into());
                        };
                        let to = self.static_int(nx).map_err(|_| {
                            "a vector size that is not known during elaboration".to_string()
                        })?;
                        if to <= 0 || to > 1 << 20 {
                            return Err("a null or very large vector size".into());
                        }
                        let to = to as u32;
                        let a = self.arg(a)?;
                        let a = self.part(a)?;
                        if !signed || to >= a.len {
                            return Ok(Val::Part(self.ext(a, to, signed)));
                        }
                        // A signed value cut: its sign element and the rightmost `to - 1`.
                        let ra = self.whole(a);
                        let t = self.tmp(to);
                        self.ops.push(Op::Extract {
                            dst: t.r,
                            src: ra,
                            lo: 0,
                            len: to,
                            full: a.len,
                        });
                        let s = self.loc(1);
                        self.ops.push(Op::Extract {
                            dst: s,
                            src: ra,
                            lo: a.len - 1,
                            len: 1,
                            full: a.len,
                        });
                        self.ops.push(Op::Insert {
                            dst: t.r,
                            src: s,
                            lo: to - 1,
                            len: 1,
                        });
                        Ok(Val::Part(t))
                    }
                    I::Shift(s) | I::ShiftFn(s) => {
                        let [a, n] = args else {
                            return Err("a shift with an unexpected number of arguments".into());
                        };
                        let a = self.arg(a)?;
                        let a = self.part(a)?;
                        let Actual::Expr(nx) = n.actual else {
                            return Err("a count that is not given".into());
                        };
                        if let Ok(k) = self.static_int(nx) {
                            return Ok(Val::Part(self.static_shift(shift(s), a, k)));
                        }
                        // A count chosen at run time: `NATURAL` (the functions) and one word.
                        if !matches!(op, I::ShiftFn(_)) || a.len > 64 {
                            return Err(
                                "a shift by a count chosen at run time of an operator or of more \
                                 than 64 elements is not supported in cycle mode yet"
                                    .into(),
                            );
                        }
                        let n = self.expr(nx)?;
                        let n = self.int(n)?;
                        self.natural(n);
                        let ra = self.whole(a);
                        let t = self.tmp(a.len);
                        self.ops.push(Op::Shift {
                            k: shift(s),
                            dst: t.r,
                            a: ra,
                            len: a.len,
                            n: n.r,
                        });
                        Ok(Val::Part(t))
                    }
                    I::Neg | I::Abs => {
                        let a = {
                            let v = one(self)?;
                            self.part(v)
                        }?;
                        let ra = self.whole(a);
                        let z = self.tmp(a.len);
                        self.ops.push(Op::Lit {
                            dst: z.r,
                            val: vec![0; words(a.len) as usize].into(),
                        });
                        let t = self.tmp(a.len);
                        self.ops.push(Op::Add {
                            dst: t.r,
                            a: z.r,
                            b: ra,
                            len: a.len,
                            sub: true,
                        });
                        if op == I::Neg {
                            return Ok(Val::Part(t));
                        }
                        let s = self.loc(1);
                        self.ops.push(Op::Extract {
                            dst: s,
                            src: ra,
                            lo: a.len - 1,
                            len: 1,
                            full: a.len,
                        });
                        let r = self.tmp(a.len);
                        self.ops.push(Op::Select {
                            dst: r.r,
                            c: s,
                            a: t.r,
                            b: ra,
                            len: a.len,
                        });
                        Ok(Val::Part(r))
                    }
                    _ => not_yet(),
                }
            }
        }
    }
}

/// The words `r` as a fresh value of `len` elements.
fn part_of(r: Ref, len: u32) -> Part {
    Part {
        r,
        lo: 0,
        len,
        full: len,
        dims: Dims::NONE,
        fresh: true,
        leaf: None,
        at: None,
        obj: None,
        lit: None,
    }
}

impl Lx<'_> {
    fn int_arith(&mut self, x: ExprId, op: Arith, a: IntV, b: IntV) -> R<IntV> {
        let (lo, hi, ty) = self.result_range(x)?;
        let site = self.site(Check::Arith { op, ty });
        let t = self.tmp_int(lo, hi);
        self.ops.push(Op::Int {
            op,
            dst: t.r,
            a: a.r,
            b: b.r,
            lo,
            hi,
            site,
        });
        Ok(t)
    }

    fn gate(&mut self, g: Gate, a: Val, b: Val) -> R<Val> {
        let (a, b) = (self.part(a)?, self.part(b)?);
        if a.len != b.len {
            return Err("a logical operator on operands of different lengths".into());
        }
        let (ra, rb) = (self.whole(a), self.whole(b));
        let t = self.tmp(a.len);
        self.ops.push(Op::Gate {
            g,
            dst: t.r,
            a: ra,
            b: rb,
            len: a.len,
        });
        Ok(Val::Part(t))
    }

    fn not(&mut self, a: Val) -> R<Val> {
        let a = self.part(a)?;
        let ra = self.whole(a);
        let t = self.tmp(a.len);
        self.ops.push(Op::Not {
            dst: t.r,
            a: ra,
            len: a.len,
        });
        Ok(Val::Part(t))
    }

    /// `and`, `or`, `nand`, `nor` of BIT and BOOLEAN evaluate the right operand only when the
    /// left does not decide (LRM 9.2.2): where it could fail a check, behind a branch.
    fn short_circuit(&mut self, g: Gate, args: &[yavs_sem::hir::Arg]) -> R<Val> {
        let [a, b] = args else {
            return Err("a logical operator with an unexpected number of arguments".into());
        };
        let a = self.arg(a)?;
        let a = self.part(a)?;
        let ra = self.whole(a);
        let outer = std::mem::take(&mut self.ops);
        let rb = self
            .arg(b)
            .and_then(|b| self.part(b))
            .map(|b| self.whole(b));
        let right = std::mem::replace(&mut self.ops, outer);
        let rb = rb?;
        let t = self.tmp(1);
        if !right.iter().any(Op::can_fail) {
            self.ops.extend(right);
            self.ops.push(Op::Gate {
                g,
                dst: t.r,
                a: ra,
                b: rb,
                len: 1,
            });
            return Ok(Val::Part(t));
        }
        // `a and b`: `b` when `a`, else false; `a or b`: true when `a`, else `b`.
        let and = matches!(g, Gate::And | Gate::Nand);
        let inv = matches!(g, Gate::Nand | Gate::Nor);
        let mut eval = right;
        eval.push(Op::Copy {
            dst: t.r,
            src: rb,
            len: 1,
        });
        let fixed = vec![Op::Lit {
            dst: t.r,
            val: Box::new([u64::from(!and)]),
        }];
        let (then, else_) = if and { (eval, fixed) } else { (fixed, eval) };
        self.ops.push(Op::If {
            cond: ra,
            then,
            else_,
        });
        if inv {
            return self.not(Val::Part(t));
        }
        Ok(Val::Part(t))
    }

    /// `a = b` (`ne`: `/=`) of logic values; `matching`: `?=` and `std_match`, where a `'-'` of
    /// a literal operand matches anything.
    fn vec_eq(&mut self, a: Val, b: Val, ne: bool, matching: bool) -> R<Val> {
        let (a, b) = (self.part(a)?, self.part(b)?);
        if a.len != b.len {
            if matching {
                return Err("a matching comparison of operands of different lengths".into());
            }
            // Arrays of different lengths are never equal (LRM 9.2.3).
            return Ok(Val::Part(self.bool_lit(ne)));
        }
        if let Some(k) = b.lit.or(a.lit) {
            // The literal's own codes decide (`'-'` as a mask when matching).
            let v = if b.lit.is_some() { a } else { b };
            let codes = self.lits[k as usize].clone();
            let r = self.match_codes(v, &codes, matching)?;
            if !ne {
                return Ok(Val::Part(part_of(r, 1)));
            }
            return self.not(Val::Part(part_of(r, 1)));
        }
        let (ra, rb) = (self.whole(a), self.whole(b));
        let t = self.tmp(1);
        self.ops.push(Op::VecEq {
            dst: t.r,
            a: ra,
            b: rb,
            len: a.len,
            ne,
        });
        Ok(Val::Part(t))
    }

    /// `numeric_std` `/`, `rem` (1) and `mod` (2): vectors of at most 64 elements, computed in
    /// one word; the result has the left operand's length for `/` and the right one's for `rem`
    /// and `mod` of two vectors, the vector's with an integer.
    fn num_div(
        &mut self,
        args: &[yavs_sem::hir::Arg],
        kind: u8,
        signed: bool,
        form: yavs_ir::ieee::Form,
    ) -> R<Val> {
        use yavs_ir::ieee::Form;
        let [a, b] = args else {
            return Err("a division with an unexpected number of arguments".into());
        };
        let (a, b) = (self.arg(a)?, self.arg(b)?);
        let (a, b, res) = match form {
            Form::Vv => {
                let (a, b) = (self.part(a)?, self.part(b)?);
                let res = if kind == 0 { a.len } else { b.len };
                (a, b, res)
            }
            Form::Vi => {
                let a = self.part(a)?;
                let b = self.int(b)?;
                if !signed {
                    self.natural(b);
                }
                let n = a.len;
                let b = self.int_vec(b, 64);
                (a, b, n)
            }
            Form::Iv => {
                let a = self.int(a)?;
                let b = self.part(b)?;
                if !signed {
                    self.natural(a);
                }
                let n = b.len;
                let a = self.int_vec(a, 64);
                (a, b, n)
            }
        };
        if a.len > 64 || b.len > 64 {
            return Err(
                "a division of vectors of more than 64 elements is not supported in cycle mode yet"
                    .into(),
            );
        }
        let (a, b) = (self.ext(a, 64, signed), self.ext(b, 64, signed));
        let (ra, rb) = (self.whole(a), self.whole(b));
        let op = match kind {
            0 => Arith::Div,
            1 => Arith::Rem,
            _ => Arith::Mod,
        };
        let site = self.site(Check::Arith {
            op,
            ty: if signed { "SIGNED" } else { "UNSIGNED" }.into(),
        });
        let t = self.tmp(64);
        self.ops.push(Op::VDiv {
            op,
            signed,
            dst: t.r,
            a: ra,
            b: rb,
            len: 64,
            site,
        });
        Ok(Val::Part(self.ext(t, res, signed)))
    }

    /// A `numeric_std` comparison: two vectors at the longer length, a vector and an integer as
    /// integers (vectors of at most 63 elements, 64 signed).
    fn num_compare(
        &mut self,
        args: &[yavs_sem::hir::Arg],
        c: Cmp,
        signed: bool,
        form: yavs_ir::ieee::Form,
    ) -> R<Val> {
        use yavs_ir::ieee::Form;
        if form == Form::Vv {
            let (a, b, n) = self.num_operands(args, form, signed)?;
            let (ra, rb) = (self.whole(a), self.whole(b));
            let t = self.tmp(1);
            self.ops.push(Op::VCmp {
                cmp: c,
                signed,
                dst: t.r,
                a: ra,
                b: rb,
                len: n,
            });
            return Ok(Val::Part(t));
        }
        let [a, b] = args else {
            return Err("a comparison with an unexpected number of arguments".into());
        };
        let (a, b) = (self.arg(a)?, self.arg(b)?);
        let (v, i, swap) = match form {
            Form::Vi => (self.part(a)?, self.int(b)?, false),
            _ => (self.part(b)?, self.int(a)?, true),
        };
        if !signed {
            self.natural(i);
        }
        if v.len > if signed { 64 } else { 63 } {
            // Both as vectors one wider than the vector (the integer fits in 64 elements).
            let n = v.len + 1;
            let v = self.ext(v, n, signed);
            let iv = self.int_vec(i, n);
            let (p, q) = if swap { (iv, v) } else { (v, iv) };
            let (rp, rq) = (self.whole(p), self.whole(q));
            let t = self.tmp(1);
            self.ops.push(Op::VCmp {
                cmp: c,
                signed: true,
                dst: t.r,
                a: rp,
                b: rq,
                len: n,
            });
            return Ok(Val::Part(t));
        }
        let rv = self.whole(v);
        let vi = self.tmp_int(i64::MIN, i64::MAX);
        self.ops.push(Op::ToInt {
            dst: vi.r,
            a: rv,
            len: v.len,
            signed,
        });
        let (p, q) = if swap { (i, vi) } else { (vi, i) };
        Ok(Val::Part(part_of(self.int_cmp(c, p, q), 1)))
    }
}

/// Every statement under `body`, nested ones included.
fn deep(db: &Db, body: &[StmtId], out: &mut Vec<StmtId>) {
    for &s in body {
        out.push(s);
        deep(db, &db.stmt_children(s), out);
    }
}

fn reversed(b: Bounds) -> Bounds {
    Bounds {
        left: b.right,
        dir: match b.dir {
            Direction::To => Direction::Downto,
            Direction::Downto => Direction::To,
        },
        right: b.left,
    }
}

/// The range of an integer (or enumeration) subtype known without elaboration values.
fn int_bounds(db: &Db, st: SubtypeId) -> Option<(i64, i64)> {
    let (l, dir, r) = db.static_bounds(st)?;
    let (l, r) = (l.as_int()?, r.as_int()?);
    Some(if dir == Direction::Downto {
        (r, l)
    } else {
        (l, r)
    })
}

/// The leaves of two records of one shape, pairwise.
fn flatten(d: &Val, s: Val, out: &mut Vec<(Val, Val)>) -> R<()> {
    match (d, s) {
        (Val::Rec(ds), Val::Rec(ss)) if ds.len() == ss.len() => {
            for (d, s) in ds.iter().zip(ss) {
                flatten(d, s, out)?;
            }
            Ok(())
        }
        (Val::Rec(_), _) | (_, Val::Rec(_)) => {
            Err("an assignment whose value has another shape than its target".into())
        }
        (d, s) => {
            out.push((d.clone(), s));
            Ok(())
        }
    }
}

/// Bits `lo .. lo + len` of the words `w` set.
fn set_bits(w: &mut [u64], lo: u32, len: u32) {
    let mut a = lo;
    while a < lo + len {
        let k = a / 64;
        let b = (lo + len).min(k * 64 + 64);
        w[k as usize] |= mask(b - a) << (a - k * 64);
        a = b;
    }
}

/// A subtype as check messages name it (the normal mode's: upper case, the base type's name for
/// an anonymous one).
fn subtype_name(db: &Db, st: SubtypeId) -> String {
    let s = &db.subtypes[st];
    match s.name {
        Some(n) => n.text.as_str().to_ascii_uppercase(),
        None => db.types[s.base].name.text.as_str().to_ascii_uppercase(),
    }
}

/// An object as check messages name it: `variable P`, `signal S`.
fn obj_desc(db: &Db, d: DeclId) -> String {
    use yavs_sem::decl::{DeclKind, ObjClass};
    let name = db.decls[d].name.to_string().to_ascii_uppercase();
    match &db.decls[d].kind {
        DeclKind::Object(o) if o.class == ObjClass::Signal => format!("signal {name}"),
        DeclKind::Object(_) => format!("variable {name}"),
        _ => name,
    }
}

/// The element (0 = rightmost) of index `i` of an array with index range `b`.
fn elem_of(b: Bounds, i: i64) -> Option<u32> {
    let n = b.len() as i64;
    let p = match b.dir {
        Direction::To => i - b.left,
        Direction::Downto => b.left - i,
    };
    (0..n).contains(&p).then(|| (n - 1 - p) as u32)
}
