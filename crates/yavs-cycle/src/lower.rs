//! The bodies of member processes, from the elaborated HIR to region operations on two-state
//! words: a logic value of `len` elements is `ceil(len / 64)` words (element 0 in bit 0, the bits
//! above `len` zero), an integer or an enumeration position one word. Metavalues read as 0: a
//! literal's are counted per site, a variable's missing initial value per element. Anything else
//! is a reason to leave the process in the event kernel, never a guess.

use std::collections::{BTreeSet, HashMap};

use yavs_diag::Span;
use yavs_elab::{Bounds, Elaborated, LeafId, ProcInst, Shape};
use yavs_sem::hir::{
    Actual, AggBounds, AttrPrefix, Choice, ChoiceSet, ExprKind, LoopScheme, PredefAttr, Rhs,
    Sensitivity, StmtKind, Target, Waveform,
};
use yavs_sem::implicit::Family;
use yavs_sem::stat::{ConstValue, Elems};
use yavs_sem::types::{Constraint, Direction, RangeC, TypeKind};
use yavs_sem::{Db, DeclId, ExprId, Id, StmtId, SubtypeId, TypeId};

use crate::codegen::{mask, words};

/// Words of the region's state: the current value of a kernel signal (all the leaves the kernel
/// keeps in it, each at its kernel offset), a slot of the process (a leaf of a variable, a
/// temporary), numbered per body, or the region's edge word (1 when the clock edge happened in
/// this wake; read by processes with an asynchronous reset).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub(crate) enum Ref {
    Sig(u32),
    Loc(u32),
    Edge,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Gate {
    And,
    Or,
    Xor,
    Nand,
    Nor,
    Xnor,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Cmp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Arith {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Rem,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
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
    /// Not a check: an assert or report of a top-level member that fired (A19); its site is a
    /// check's, so a site names the member.
    Note(Box<Note>),
}

/// An assert or report of a process of the top-level unit, evaluated in region code (user
/// decision A19): its severity, its message as pieces and the words that hold the values the
/// pieces format, copied there where the statement ran (a member's slots; image words once
/// `lib.rs` placed the member).
#[derive(Clone, Debug)]
pub(crate) struct Note {
    pub severity: u8,
    pub pieces: Vec<Piece>,
    pub args: Vec<(Ref, u32)>,
}

/// A piece of a report's message.
#[derive(Clone, Debug)]
pub(crate) enum Piece {
    Text(Box<[u8]>),
    /// The integer in the words of arg `n`, in decimal.
    Int(usize),
    /// The enumeration value in the words of arg `n` (a position, or a logic value's 0 or 1):
    /// the text of each.
    Enum(usize, Box<[Box<[u8]>]>),
    /// The `len` elements of arg `n`, leftmost first, `bits` per digit (1, 3 or 4).
    Bits {
        n: usize,
        len: u32,
        bits: u32,
    },
}

impl Note {
    /// The message, from the image words of the args.
    pub(crate) fn text(&self, img: &[u64]) -> String {
        let mut out: Vec<u8> = Vec::new();
        let word = |n: usize, k: u32| match self.args[n].0 {
            Ref::Loc(w) => img[(w + k) as usize],
            _ => 0,
        };
        for p in &self.pieces {
            match p {
                Piece::Text(t) => out.extend_from_slice(t),
                Piece::Int(n) => out.extend(yavs_std::image::integer_image(word(*n, 0) as i64)),
                Piece::Enum(n, names) => {
                    let v = word(*n, 0) as usize;
                    out.extend_from_slice(names.get(v).map_or(&b"?"[..], |t| &t[..]));
                }
                Piece::Bits { n, len, bits } => {
                    let bit = |i: u32| (word(*n, i / 64) >> (i % 64)) & 1;
                    let digits = len.div_ceil(*bits);
                    for d in (0..digits).rev() {
                        let mut v = 0;
                        for b in (0..*bits).rev() {
                            let i = d * bits + b;
                            v = v << 1 | if i < *len { bit(i) } else { 0 };
                        }
                        out.push(b"0123456789ABCDEF"[v as usize]);
                    }
                }
            }
        }
        out.iter().map(|&b| b as char).collect()
    }
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
            } if what.is_empty() => {
                format!("value {a} outside of {ty} range {}", range(*lo, *hi, *down))
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
            Check::Note(_) => "an assert or report fired".into(),
        }
    }
}

/// One operation on words. Every operand is read before the result is written, so a result may
/// be an operand. `len` is in elements; a 1-bit value is a boolean or a scalar logic value.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
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
    /// Report `site` (a [`Check::Note`]) fired: its args' words into memory and the site into
    /// the region's log; only in a member's last run (no effect where checks do not exit).
    Note {
        site: u32,
        args: Vec<(Ref, u32)>,
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
            | Op::Note { .. }
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
            Op::Note { site: x, args } => {
                *x = site(*x);
                for a in args {
                    a.0 = s(a.0);
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

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
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
    /// A process of the top-level unit: why an assert or report of it cannot be evaluated in
    /// region code (with `dropped` above 0), and how many are (`Op::Note`).
    pub note_why: Option<String>,
    pub notes: u32,
    /// Literals with a metavalue (`'U'`, `'X'`, `'W'`, `'-'`), read as 0.
    pub metas: u32,
    /// Elements of logic variables without an initial value (`'U'`), which start at 0.
    pub uninit: u64,
    /// The checks of the body, numbered by `site`, each with the source place of the statement
    /// (an operation for arithmetic) it belongs to, then the statements of the calls it is inlined
    /// in, innermost first.
    pub checks: Vec<(Check, Vec<Span>)>,
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
    /// An array of no elements, of these index ranges (one of them null): nothing to read or
    /// write; only assigned a null value, indexed, sliced, an operand of `&` or of `resize`.
    Null(Dims),
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
    /// rightmost of a slice), the array's index range, the width of one of its elements, and where
    /// in that element the part starts (a field or slice of the element); the part is `len` wide.
    at: Option<(Ref, Bounds, u32, u32)>,
    /// The object a name starts from (messages of index checks).
    obj: Option<DeclId>,
    /// A literal with a metavalue element: its `std_ulogic` codes (`Lx::lits`), since no
    /// two-state comparison means what the comparison with it means in VHDL.
    lit: Option<u32>,
    /// An array of integers or enumeration positions: one word per element (the innermost index
    /// range is the word's, `WORD`), each element of this range.
    int: Option<IntElem>,
}

/// The elements of an array of integers or enumerations: the values they hold, and the subtype
/// and object an assignment to one is checked against.
#[derive(Clone, Copy, Debug)]
struct IntElem {
    lo: i64,
    hi: i64,
    down: bool,
    place: Option<(SubtypeId, DeclId)>,
}

/// The index range of the bits of one word: the innermost range of an array of integers.
const WORD: Bounds = Bounds {
    left: 63,
    dir: Direction::Downto,
    right: 0,
};

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

/// What a branch of an `if`, `case` or a conditional or selected assignment does.
#[derive(Clone, Copy)]
enum Br<'b> {
    Stmts(&'b [StmtId]),
    Wave(&'b Target, &'b Waveform),
    Var(&'b Target, ExprId),
}

/// A call being inlined: its formals and locals, and its result.
struct Frame {
    callee: DeclId,
    binds: HashMap<DeclId, Bind>,
    /// The result subtype of a function.
    ret_st: Option<SubtypeId>,
    /// The result: the place every `return` writes, or (`direct`) the value of the one `return`
    /// that ends the body.
    ret: Option<Val>,
    direct: bool,
    /// The statement the call is in.
    from: Span,
    /// Set by a `return` that the rest of the body must not run after.
    flag: Option<Ref>,
}

/// What a formal, a local constant or a local variable of an inlined call is.
#[derive(Clone)]
enum Bind {
    /// A scalar known during elaboration.
    Const(ConstValue),
    Val(Val),
}

/// Where a `return` (of the call at that depth), `exit` or `next` (of that loop) goes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tgt {
    Ret(usize),
    Exit(StmtId),
    Next(StmtId),
}

/// A jump the path may have taken: `always` when every path did. Its flag (1 when taken) is
/// what the statements after it test; a jump with no flag is always taken where it is.
#[derive(Clone, Copy, Debug)]
struct Jump {
    tgt: Tgt,
    flag: Option<Ref>,
    always: bool,
}

/// The jumps after one of two branches ran.
fn merge_branches(a: Vec<Jump>, b: Vec<Jump>) -> Vec<Jump> {
    let mut out: Vec<Jump> = Vec::new();
    for j in a.iter().chain(&b) {
        if !out.iter().any(|o| o.tgt == j.tgt) {
            out.push(*j);
        }
    }
    for o in &mut out {
        let always = |x: &[Jump]| x.iter().any(|j| j.tgt == o.tgt && j.always);
        o.always = always(&a) && always(&b);
    }
    out
}

/// The jumps after `saved` may have been taken and, only when none was, code that took `rest`:
/// a jump is certain when the code certainly took it and every earlier one was the same.
fn merge_guarded(saved: Vec<Jump>, rest: Vec<Jump>) -> Vec<Jump> {
    let mut out = saved.clone();
    for j in &rest {
        if !out.iter().any(|o| o.tgt == j.tgt) {
            out.push(*j);
        }
    }
    for o in &mut out {
        o.always =
            rest.iter().any(|j| j.tgt == o.tgt && j.always) && saved.iter().all(|s| s.tgt == o.tgt);
    }
    out
}

/// Most calls inlined into one another.
const DEPTH_MAX: usize = 32;

/// Why an assert or report of a top-level process outside a clock edge is not evaluated in
/// region code: the region settles once per wake where the normal mode runs the process at
/// every event of what it waits on.
const NOTE_COMB: &str = "an assert or report that runs outside a clock edge";

/// Why a comparison or arithmetic with a literal holding a metavalue keeps its process out.
const META_CMP: &str = "a comparison with a metavalue literal (`'U'`, `'X'`, `'-'`, ...), which two-state \
                        values never equal";

/// Most elements of a logic value: 2**26 (a megaword), so element offsets stay far inside `u32`
/// (a run-time index scales them by an element's width in `i64`). A memory that large is not
/// copied or exported per wake (`region::ram_rule`, `rt::RamWrite`).
const VEC_MAX: u64 = 1 << 26;

/// Most statement instances an unrolled loop makes per process.
const UNROLL_MAX: u32 = 1 << 16;

/// Lower process `pi` if its shape and every construct of its body are in the subset.
pub(crate) fn lower(db: &Db, e: &Elaborated, pi: usize) -> R<Body> {
    let p = &e.procs[pi];
    if p.postponed {
        return Err("a postponed process cannot run cycle-based".into());
    }
    let mut lx = Lx {
        db,
        e,
        p,
        vars: HashMap::new(),
        consts: HashMap::new(),
        known: HashMap::new(),
        frames: Vec::new(),
        pend: Vec::new(),
        loop_flags: HashMap::new(),
        ops: Vec::new(),
        init_ops: Vec::new(),
        reads: BTreeSet::new(),
        rparts: Vec::new(),
        writes: BTreeSet::new(),
        locs: 0,
        init: Vec::new(),
        var_words: Vec::new(),
        dropped: 0,
        top: crate::region::in_top_unit(e, pi),
        at_edge: false,
        note_why: None,
        notes: 0,
        metas: 0,
        uninit: 0,
        checks: Vec::new(),
        at: db.stmts[p.stmt].span,
        unrolled: 0,
        lits: Vec::new(),
        bounds: false,
    };
    if let Some(pa) = &p.actual {
        lx.port_actual(pa)?;
        return Ok(lx.body(Kind::Comb, None, None));
    }
    match &db.stmts[p.stmt].kind {
        StmtKind::Process {
            sensitivity, body, ..
        } => lx.process(sensitivity, body),
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
            if lx.top {
                lx.note_why = Some(NOTE_COMB.into());
            }
            Ok(lx.body(Kind::Comb, None, None))
        }
        StmtKind::ConcProcCall {
            postponed: false,
            call,
            ..
        } => {
            lx.proc_call(call)?;
            Ok(lx.body(Kind::Comb, None, None))
        }
        _ => Err("a concurrent statement of this kind is not supported in cycle mode yet".into()),
    }
}

struct Lx<'a> {
    /// A name is being looked at for its index range only (`name_bounds`): a constant may be
    /// named as if it were a target.
    bounds: bool,
    db: &'a Db,
    e: &'a Elaborated,
    p: &'a ProcInst,
    /// The leaves of each process variable met so far.
    vars: HashMap<DeclId, Val>,
    /// The parameters of the loops being unrolled, with their value in this iteration.
    consts: HashMap<DeclId, ConstValue>,
    /// Integer and enumeration variables (process variables, and locals and formals of inlined
    /// calls) whose value is known at this point of the body, by their word: assigned a value
    /// elaboration knows on every path here. Reads of them fold (`lo := 0; hi := lo + 7;
    /// v(hi downto lo)` is a static slice).
    known: HashMap<u32, ConstValue>,
    /// The calls being inlined, the innermost last.
    frames: Vec<Frame>,
    /// The jumps (`return`, `exit`, `next`) the current path may have taken since the start of
    /// the statement list being lowered: while one may have, the rest runs behind its flag.
    pend: Vec<Jump>,
    /// The `exit` and `next` flags of the loops being unrolled.
    loop_flags: HashMap<StmtId, (Option<Ref>, Option<Ref>)>,
    ops: Vec<Op>,
    init_ops: Vec<Op>,
    reads: BTreeSet<LeafId>,
    rparts: Vec<(LeafId, u32, u32)>,
    writes: BTreeSet<LeafId>,
    locs: u32,
    init: Vec<(u32, u64)>,
    var_words: Vec<(u32, u32)>,
    dropped: u32,
    /// A process of the top-level unit: its asserts and reports are evaluated (A19) where they
    /// can be: at the clock edge of a clocked process (`at_edge`).
    top: bool,
    at_edge: bool,
    note_why: Option<String>,
    notes: u32,
    metas: u32,
    uninit: u64,
    checks: Vec<(Check, Vec<Span>)>,
    /// The place new check sites report: the statement being lowered.
    at: Span,
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
            note_why: self.note_why,
            notes: self.notes,
            metas: self.metas,
            uninit: self.uninit,
            checks: self.checks,
        }
    }

    fn process(mut self, sensitivity: &Sensitivity, body: &[StmtId]) -> R<Body> {
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
                    let then = self.capture(|lx| lx.stmts(a))?;
                    let at_edge = self.capture(|lx| lx.edge_body(*cond, b))?;
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
        // The subset checker proved the rest (`check.rs`): every element a path may assign is
        // assigned on every path, every variable is assigned before it is read, and the
        // sensitivity list names everything the body reads.
        self.stmts(body)?;
        // A combinational variable is assigned before any read: its start value is never seen.
        self.uninit = 0;
        self.init_ops.clear();
        Ok(self.body(Kind::Comb, None, None))
    }

    /// The body of a clocked process under its edge condition `cond`: the edge itself, or the
    /// edge `and` a condition that decides whether the body runs at the edge.
    fn edge_body(&mut self, cond: ExprId, body: &[StmtId]) -> R<()> {
        self.at_edge = true;
        let r = match self.edge_rest(cond)? {
            None => self.stmts(body),
            Some(c) => self.if_chain(&[(Some(c), Br::Stmts(body))]),
        };
        self.at_edge = false;
        r
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
        Ok(self.body(Kind::Comb, None, None))
    }

    /// The equivalent process `formal <= expr` of a port's expression actual (LRM 6.5.6.3): the
    /// whole formal, of the instance, takes the expression's value, evaluated in the
    /// instantiating region.
    fn port_actual(&mut self, pa: &yavs_elab::PortActual) -> R<()> {
        let nets = &self.e.nets;
        let &d0 = (self.p.drivers.first()).ok_or("a port expression actual that drives nothing")?;
        let so = nets.obj(nets.leaf(nets.drivers[d0].leaf).obj);
        if so.decl != pa.port || so.shape != pa.shape {
            return Err(
                "an expression actual of part of a port is not supported in cycle mode yet".into(),
            );
        }
        let mut next = so.leaves.0.0;
        let v = self
            .tree(&so.shape, so.decl, &mut |_| {
                next += 1;
                Ref::Loc(next - 1)
            })
            .map_err(|w| format!("{w} (port `{}`)", so.name.text))?;
        if next != so.leaves.1.0 {
            return Err("a port whose leaves are not one per field".into());
        }
        let dst = self.place_sig(v, so)?;
        let src = self.expr_h(pa.expr, Some(&dst))?;
        self.assign(&dst, src)
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
            int: None,
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
        let calls = self.frames.iter().rev().map(|f| f.from);
        self.checks
            .push((c, std::iter::once(self.at).chain(calls).collect()));
        self.checks.len() as u32 - 1
    }

    /// The operations `f` makes, apart, as code that may or may not run: the current list is
    /// restored whatever `f` returns, and only the variable values known both before and after
    /// it stay known.
    fn capture(&mut self, f: impl FnOnce(&mut Self) -> R<()>) -> R<Vec<Op>> {
        let (ops, after) = self.capture_known(f)?;
        self.known.retain(|w, c| after.get(w) == Some(c));
        Ok(ops)
    }

    /// The same, giving the values known after `f` and leaving those known before it.
    fn capture_known(
        &mut self,
        f: impl FnOnce(&mut Self) -> R<()>,
    ) -> R<(Vec<Op>, HashMap<u32, ConstValue>)> {
        let outer = std::mem::take(&mut self.ops);
        let before = self.known.clone();
        let r = f(self);
        let inner = std::mem::replace(&mut self.ops, outer);
        let after = std::mem::replace(&mut self.known, before);
        r.map(|()| (inner, after))
    }

    /// The values known after one of two branches ran.
    fn join_known(&mut self, a: HashMap<u32, ConstValue>, b: &HashMap<u32, ConstValue>) {
        self.known = a;
        self.known.retain(|w, c| b.get(w) == Some(c));
    }

    /// A statement list. After a statement that may have jumped (`return`, `exit`, `next`), each
    /// later one runs behind the flags of the jumps (a flag, once set, stays set for the rest of
    /// the list); after one that jumped for sure, the rest is never reached.
    fn stmts(&mut self, body: &[StmtId]) -> R<()> {
        for &s in body {
            if self.pend.iter().any(|j| j.always) {
                break;
            }
            if self.pend.is_empty() {
                self.stmt(s)?;
            } else {
                self.guard(|lx| lx.stmt(s))?;
            }
        }
        Ok(())
    }

    /// `f` behind the flags of the jumps that may have been taken: it runs when none was.
    fn guard(&mut self, f: impl FnOnce(&mut Self) -> R<()>) -> R<()> {
        let mut flags: Vec<Ref> = Vec::new();
        for j in &self.pend {
            let f = j.flag.ok_or("a jump cycle mode cannot follow")?;
            if !flags.contains(&f) {
                flags.push(f);
            }
        }
        let mut cond = flags[0];
        for &f in &flags[1..] {
            let o = self.tmp(1);
            self.ops.push(Op::Gate {
                g: Gate::Or,
                dst: o.r,
                a: cond,
                b: f,
                len: 1,
            });
            cond = o.r;
        }
        let saved = std::mem::take(&mut self.pend);
        let r = self.capture(f);
        let inner = std::mem::take(&mut self.pend);
        self.pend = merge_guarded(saved, inner);
        self.ops.push(Op::If {
            cond,
            then: Vec::new(),
            else_: r?,
        });
        Ok(())
    }

    /// Statement `s`; a reason it cannot be lowered names the line of the innermost statement.
    fn stmt(&mut self, s: StmtId) -> R<()> {
        let outer = std::mem::replace(&mut self.at, self.db.stmts[s].span);
        let r = self.stmt_kind(s).map_err(|w| {
            if w.ends_with(')') && w.contains(".vhd") {
                return w;
            }
            let span = self.db.stmts[s].span;
            let f = self.db.sources.get(span.file);
            let file = std::path::Path::new(&f.path)
                .file_name()
                .map_or(f.path.clone(), |n| n.to_string_lossy().into_owned());
            format!("{w} ({file}:{})", f.line_col(span.start).0)
        });
        self.at = outer;
        r
    }

    fn stmt_kind(&mut self, s: StmtId) -> R<()> {
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
            } => self.for_loop(s, *param, range, body),
            StmtKind::Loop { .. } => Err(
                "a `while` loop or a loop without a scheme is not supported in cycle mode yet"
                    .into(),
            ),
            StmtKind::Next { loop_, when } => self.jump(Tgt::Next(*loop_), *when),
            StmtKind::Exit { loop_, when } => self.jump(Tgt::Exit(*loop_), *when),
            StmtKind::Wait { .. } => Err("a wait statement runs in the event kernel".into()),
            // Simulation-only: compiled to nothing, counted for the report (the mode is for speed).
            StmtKind::Assert { .. } | StmtKind::Report { .. } if !self.top => {
                self.dropped += 1;
                Ok(())
            }
            // A process of the top-level unit (A19): evaluated at the clock edge, else it
            // keeps the process in the event kernel (`region::select`).
            StmtKind::Assert { .. } | StmtKind::Report { .. } => {
                let made = if self.at_edge {
                    self.capture_known(|lx| lx.note(s)).map(|(ops, _)| ops)
                } else {
                    Err(NOTE_COMB.into())
                };
                match made {
                    Ok(ops) => {
                        self.ops.extend(ops);
                        self.notes += 1;
                    }
                    Err(w) => {
                        self.dropped += 1;
                        self.note_why.get_or_insert(w);
                    }
                }
                Ok(())
            }
            StmtKind::ProcCall(call) => self.proc_call(call),
            StmtKind::Force { .. } | StmtKind::Release { .. } => {
                Err("`force` and `release` cannot run cycle-based".into())
            }
            StmtKind::Return(v) => self.ret(*v),
            _ => Err("a statement that is not supported in cycle mode yet".into()),
        }
    }

    fn var_assign(&mut self, target: &Target, v: ExprId) -> R<()> {
        if self.null_slice(target.expr) && self.null_slice(v) {
            return Ok(());
        }
        let dst = self.name(target.expr, true)?;
        if self.signal_rooted(&dst) {
            return Err("an assignment target that is not a variable".into());
        }
        let c = self.fold(v);
        let src = self.expr_h(v, Some(&dst))?;
        self.assign(&dst, src)?;
        self.learn(&dst, c);
        Ok(())
    }

    /// After `dst := x` (`c` the value of `x` when elaboration knows it, folded before the
    /// assignment): `dst`'s value, when it is a whole integer or enumeration variable (`assign`
    /// forgot what it held before).
    fn learn(&mut self, dst: &Val, c: Option<ConstValue>) {
        if let Val::Int(IntV { r: Ref::Loc(w), .. }) = dst
            && let Some(c @ (ConstValue::Int(_) | ConstValue::Enum(_))) = c
        {
            self.known.insert(*w, c);
        }
    }

    /// The value of the integer or enumeration variable at `v` when it is known here.
    fn known_val(&self, v: &Val) -> Option<ConstValue> {
        match v {
            Val::Int(IntV { r: Ref::Loc(w), .. }) => self.known.get(w).cloned(),
            _ => None,
        }
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
        // A null slice given a null slice (`p(2 to n) <= p(1 to n - 1)` with n = 1): nothing to
        // assign.
        if self.null_slice(target.expr) && self.null_slice(v) {
            return Ok(());
        }
        let dst = self.name(target.expr, true)?;
        if !self.signal_rooted(&dst) && !all_null(&dst) {
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
        let (then, kt) = self.capture_known(|lx| lx.branch(*b))?;
        let pt = std::mem::take(&mut self.pend);
        let (else_, ke) = self.capture_known(|lx| lx.if_chain(rest))?;
        let pe = std::mem::take(&mut self.pend);
        self.pend = merge_branches(pt, pe);
        self.join_known(kt, &ke);
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
        let (then, kt) = self.capture_known(|lx| lx.branch(*b))?;
        let pt = std::mem::take(&mut self.pend);
        let (else_, ke) = self.capture_known(|lx| lx.ref_chain(rest))?;
        let pe = std::mem::take(&mut self.pend);
        self.pend = merge_branches(pt, pe);
        self.join_known(kt, &ke);
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
                _ => return Err(META_CMP.into()),
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
    /// with `i` a constant. After an iteration that may have left the loop (`exit`, or a jump
    /// out of it), every later one runs behind the flags.
    fn for_loop(&mut self, s: StmtId, param: DeclId, range: &RangeC, body: &[StmtId]) -> R<()> {
        let db = self.db;
        let mut inner = Vec::new();
        deep(db, body, &mut inner);
        if (inner.iter()).any(|&x| matches!(db.stmts[x].kind, StmtKind::Wait { .. })) {
            return Err("a loop with a `wait` runs in the event kernel".into());
        }
        let targets = |exit: bool| {
            inner.iter().any(|&x| match db.stmts[x].kind {
                StmtKind::Exit { loop_, .. } => exit && loop_ == s,
                StmtKind::Next { loop_, .. } => !exit && loop_ == s,
                _ => false,
            })
        };
        let (b, _) = self.static_range(range)?;
        let n = b.len();
        let size = (inner.len() as u64 + 1).saturating_mul(n);
        if size + u64::from(self.unrolled) > u64::from(UNROLL_MAX) {
            return Err(format!(
                "a loop of {n} iterations, above what cycle mode unrolls ({UNROLL_MAX} statements)"
            ));
        }
        self.unrolled += size as u32;
        let exit = targets(true).then(|| self.loc(1));
        let next = targets(false).then(|| self.loc(1));
        if let Some(f) = exit {
            self.ops.push(Op::Lit {
                dst: f,
                val: Box::new([0]),
            });
        }
        let yavs_sem::decl::DeclKind::Object(po) = &db.decls[param].kind else {
            return Err("a loop parameter that is not an object".into());
        };
        let enum_param = matches!(db.types[db.base(po.subtype)].kind, TypeKind::Enum { .. });
        let old = self.consts.get(&param).cloned();
        let old_flags = self.loop_flags.insert(s, (exit, next));
        let r: R<()> = (|| {
            for k in 0..n as i64 {
                if self.pend.iter().any(|j| j.always) {
                    break;
                }
                let i = match b.dir {
                    Direction::To => b.left + k,
                    Direction::Downto => b.left - k,
                };
                let v = if enum_param {
                    ConstValue::Enum(i as u32)
                } else {
                    ConstValue::Int(i)
                };
                self.consts.insert(param, v);
                if let Some(f) = next {
                    self.ops.push(Op::Lit {
                        dst: f,
                        val: Box::new([0]),
                    });
                }
                let iteration = |lx: &mut Self| {
                    lx.stmts(body)?;
                    lx.pend.retain(|j| j.tgt != Tgt::Next(s));
                    Ok(())
                };
                if self.pend.is_empty() {
                    iteration(self)?;
                } else {
                    self.guard(iteration)?;
                }
            }
            Ok(())
        })();
        match old {
            Some(v) => self.consts.insert(param, v),
            None => self.consts.remove(&param),
        };
        match old_flags {
            Some(f) => self.loop_flags.insert(s, f),
            None => self.loop_flags.remove(&s),
        };
        r?;
        // Leaving the loop ends its `exit`.
        self.pend.retain(|j| j.tgt != Tgt::Exit(s));
        Ok(())
    }

    /// `next` or `exit` of a loop being unrolled, if `when` holds.
    fn jump(&mut self, tgt: Tgt, when: Option<ExprId>) -> R<()> {
        let (Tgt::Exit(l) | Tgt::Next(l)) = tgt else {
            return Err("a jump cycle mode cannot follow".into());
        };
        let Some(&(exit, next)) = self.loop_flags.get(&l) else {
            return Err("`exit` or `next` of a loop that is not unrolled".into());
        };
        let flag = if matches!(tgt, Tgt::Exit(_)) {
            exit
        } else {
            next
        };
        let set = |lx: &mut Self| {
            if let Some(f) = flag {
                lx.ops.push(Op::Lit {
                    dst: f,
                    val: Box::new([1]),
                });
            }
            lx.pend.push(Jump {
                tgt,
                flag,
                always: true,
            });
        };
        let Some(c) = when else {
            set(self);
            return Ok(());
        };
        match self.fold_bool(c) {
            Some(true) => set(self),
            Some(false) => {}
            None => {
                let cond = self.cond(c)?;
                let then = self.capture(|lx| {
                    set(lx);
                    Ok(())
                })?;
                for j in &mut self.pend {
                    j.always = false;
                }
                self.ops.push(Op::If {
                    cond,
                    then,
                    else_: Vec::new(),
                });
            }
        }
        Ok(())
    }

    /// A range known during elaboration (`a to b`, `x'range`), and its type.
    fn static_range(&mut self, r: &RangeC) -> R<(Bounds, Option<TypeId>)> {
        match r {
            RangeC::Explicit { left, dir, right } => {
                let b = match (self.static_int(*left), self.static_int(*right)) {
                    (Ok(l), Ok(r)) => Bounds {
                        left: l,
                        dir: *dir,
                        right: r,
                    },
                    // The range of a type or subtype declared in a design unit, which
                    // elaboration evaluated once per instance (LRM 14.4.2.1): a function of
                    // generics, say, that folding does not run.
                    (Err(w), _) | (_, Err(w)) => self.e.range(self.p.scope, *left).ok_or(w)?,
                };
                Ok((b, Some(self.db.base(self.db.exprs[*left].subtype))))
            }
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
        let (n, l, k) = (self.ops.len(), self.locs, self.known.clone());
        let was = std::mem::replace(&mut self.bounds, true);
        let v = self.name(x, true);
        self.bounds = was;
        self.ops.truncate(n);
        self.locs = self.locs.max(l);
        // The code is dropped: so is what it would have made known.
        self.known = k;
        match v? {
            Val::Part(Part { at: Some(_), .. }) => {
                Err("an attribute of an element chosen at run time".into())
            }
            v => outer(&v).ok_or_else(|| {
                "an attribute of something that is not an array with a known range".into()
            }),
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

    /// Assert or report `s` of a process of the top-level unit, evaluated here (A19): the
    /// condition, then the values of its message copied into words of their own, then the note
    /// into the region's log.
    fn note(&mut self, s: StmtId) -> R<()> {
        let db = self.db;
        let (cond, msg, sev, default) = match &db.stmts[s].kind {
            StmtKind::Assert {
                cond,
                report,
                severity,
            } => (Some(*cond), *report, *severity, 2),
            StmtKind::Report { report, severity } => (None, Some(*report), *severity, 0),
            _ => return Err("a statement that is not an assert or report".into()),
        };
        let severity = match sev {
            None => default,
            Some(x) => match self.fold(x) {
                Some(ConstValue::Enum(p)) if p < 4 => p as u8,
                _ => return Err("a severity that is not known during elaboration".into()),
            },
        };
        let c = cond.map(|c| self.cond(c)).transpose()?;
        let (mut pieces, mut args) = (Vec::new(), Vec::new());
        // The message is evaluated only when the assertion fails, as in the normal mode.
        let mut made = Vec::new();
        if let Some(m) = msg {
            made = self.capture(|lx| lx.message(m, &mut pieces, &mut args))?;
        } else {
            pieces.push(Piece::Text(b"Assertion violation."[..].into()));
        }
        let site = self.site(Check::Note(Box::new(Note {
            severity,
            pieces,
            args: args.clone(),
        })));
        made.push(Op::Note { site, args });
        match c {
            Some(c) => self.ops.push(Op::If {
                cond: c,
                then: Vec::new(),
                else_: made,
            }),
            None => self.ops.extend(made),
        }
        Ok(())
    }

    /// The pieces of message `x` (a string): literal text, concatenations, and `'image`,
    /// `to_string`, `to_hstring` and `to_ostring` of a value, which is copied into words of its
    /// own in `args` here.
    fn message(&mut self, x: ExprId, out: &mut Vec<Piece>, args: &mut Vec<(Ref, u32)>) -> R<()> {
        let db = self.db;
        let cannot = || Err("a report message cycle mode cannot format in region code".into());
        let ty = db.base(db.exprs[x].subtype);
        if let Some(c) = self.fold(x) {
            return match c {
                ConstValue::Array(a) => match &a.elems {
                    Elems::Bytes(b) => {
                        out.push(Piece::Text(b.clone()));
                        Ok(())
                    }
                    Elems::Runs(_) => cannot(),
                },
                // A character of a concatenation.
                ConstValue::Enum(p) if Some(ty) == db.well_known.character => {
                    out.push(Piece::Text(Box::new([p as u8])));
                    Ok(())
                }
                _ => cannot(),
            };
        }
        match &db.exprs[x].kind {
            ExprKind::Qualified { operand, .. } => self.message(*operand, out, args),
            ExprKind::Attr {
                attr: PredefAttr::Image,
                prefix: AttrPrefix::Subtype(_),
                args: a,
            } if a.len() == 1 => self.value_piece(a[0], true, out, args),
            ExprKind::Call {
                callee,
                args: cargs,
                method_of: None,
                ..
            } => {
                let mut callee = *callee;
                while let yavs_sem::decl::DeclKind::Alias(yavs_sem::decl::Alias::NonObject {
                    target,
                }) = &db.decls[callee].kind
                {
                    callee = *target;
                }
                let Some(sp) = db.decls[callee].kind.subprogram() else {
                    return cannot();
                };
                let arg = |k: usize| match cargs.get(k).map(|a| &a.actual) {
                    Some(Actual::Expr(e)) => Ok(*e),
                    _ => {
                        Err("a report message cycle mode cannot format in region code".to_string())
                    }
                };
                let digits = |lx: &mut Self, bits: u32, out: &mut Vec<Piece>, args: &mut Vec<_>| {
                    let a = arg(0)?;
                    lx.bits_piece(a, bits, out, args)
                };
                match &sp.body {
                    yavs_sem::decl::Body::Implicit(op)
                        if cargs.len() == 2 && op.family == Family::Cat =>
                    {
                        self.message(arg(0)?, out, args)?;
                        self.message(arg(1)?, out, args)
                    }
                    yavs_sem::decl::Body::Implicit(op) if cargs.len() == 1 => match op.family {
                        Family::ToString => self.value_piece(arg(0)?, false, out, args),
                        Family::ToHString => digits(self, 4, out, args),
                        Family::ToOString => digits(self, 3, out, args),
                        _ => cannot(),
                    },
                    _ => match yavs_ir::ieee::native_symbol(sp.mangled.as_str())
                        .as_deref()
                        .and_then(yavs_ir::ieee::symbol_key)
                        .and_then(yavs_ir::ieee::parse)
                    {
                        Some(yavs_ir::ieee::Op::Str(octal)) if cargs.len() == 1 => {
                            digits(self, if octal { 3 } else { 4 }, out, args)
                        }
                        _ => cannot(),
                    },
                }
            }
            _ => cannot(),
        }
    }

    /// `x'image` (`image`) or `to_string(x)` of a scalar, or `to_string` of a vector of logic
    /// or bit elements.
    fn value_piece(
        &mut self,
        x: ExprId,
        image: bool,
        out: &mut Vec<Piece>,
        args: &mut Vec<(Ref, u32)>,
    ) -> R<()> {
        let db = self.db;
        let ty = db.base(db.exprs[x].subtype);
        let lit = |d: DeclId| match db.decls[d].name {
            yavs_sem::decl::Designator::Char(c) => {
                let c = c as u8;
                if image {
                    yavs_std::image::character_image(c)
                } else {
                    yavs_std::image::character_to_string(c)
                }
            }
            n => yavs_std::image::identifier_image(n.to_string().as_bytes()),
        };
        match &db.types[ty].kind {
            TypeKind::Integer | TypeKind::UniversalInteger => {
                let v = self.expr(x)?;
                let i = self.int(v)?;
                let r = self.keep_arg(i.r, 64, args);
                out.push(Piece::Int(r));
                Ok(())
            }
            TypeKind::Enum { lits } => {
                let v = self.expr(x)?;
                let all = || -> Box<[Box<[u8]>]> { lits.iter().map(|&d| lit(d).into()).collect() };
                let (r, len, names) = match v {
                    Val::Int(i) => (i.r, 64, all()),
                    // A logic value is its element's 0 or 1: BIT and BOOLEAN their two
                    // literals, STD_ULOGIC '0' and '1'.
                    Val::Part(p) if p.len == 1 => {
                        let names = if lits.len() == 2 {
                            all()
                        } else {
                            let at = |c: char| {
                                (lits.iter()).find(|&&d| {
                                    matches!(db.decls[d].name, yavs_sem::decl::Designator::Char(x) if x == c)
                                })
                            };
                            let (Some(&z), Some(&o)) = (at('0'), at('1')) else {
                                return Err("a logic value of an unknown type in a report".into());
                            };
                            Box::new([lit(z).into(), lit(o).into()])
                        };
                        let p = self.materialise(p);
                        (self.whole(p), 1, names)
                    }
                    _ => return Err("an enumeration value of another form in a report".into()),
                };
                let n = self.keep_arg(r, len, args);
                out.push(Piece::Enum(n, names));
                Ok(())
            }
            TypeKind::Array { .. } if !image => self.bits_piece(x, 1, out, args),
            _ => Err(
                "a value of this type in a report message, which cycle mode cannot format \
                      in region code"
                    .into(),
            ),
        }
    }

    /// The digits of a vector of logic or bit elements, `bits` elements each.
    fn bits_piece(
        &mut self,
        x: ExprId,
        bits: u32,
        out: &mut Vec<Piece>,
        args: &mut Vec<(Ref, u32)>,
    ) -> R<()> {
        let v = self.expr(x)?;
        let p = self.part(v)?;
        if p.int.is_some() || p.len == 0 {
            return Err("a vector in a report message cycle mode cannot format".into());
        }
        let len = p.len;
        let w = self.whole(p);
        let n = self.keep_arg(w, len, args);
        out.push(Piece::Bits { n, len, bits });
        Ok(())
    }

    /// The value in `r` (`len` elements) copied into words of its own, as arg `n` of a note.
    fn keep_arg(&mut self, r: Ref, len: u32, args: &mut Vec<(Ref, u32)>) -> usize {
        let t = self.loc(words(len));
        self.ops.push(Op::Copy {
            dst: t,
            src: r,
            len,
        });
        args.push((t, words(len)));
        args.len() - 1
    }

    fn signal_rooted(&self, v: &Val) -> bool {
        match v {
            Val::Part(p) => matches!(p.r, Ref::Sig(_)),
            Val::Int(i) => matches!(i.r, Ref::Sig(_)),
            Val::Rec(fs) => fs.iter().any(|f| self.signal_rooted(f)),
            Val::Null(_) => false,
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
                if let Some((idx, b, stride, _)) = d.at {
                    // A memory (a big signal written here) is the region's to judge
                    // (`region::ram_rule`, `build`'s pending writes).
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
            (Val::Null(_), Val::Null(_)) => Ok(()),
            (Val::Int(d), Val::Int(s)) => {
                if let Some(l) = d.leaf {
                    self.writes.insert(l);
                }
                if let Ref::Loc(w) = d.r {
                    self.known.remove(&w);
                }
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
            // An element of an array of integers read (a field of a record element of a constant
            // table chosen at run time).
            (Val::Int(_), Val::Part(s)) if s.dims.n == 1 && s.int.is_some() => {
                let s = self.materialise(s);
                let v = self.int_elem(s);
                self.assign(dst, v)
            }
            // An element of an array of integers.
            (Val::Part(d), Val::Int(s)) if d.dims.n == 1 && d.int.is_some() => {
                let w = self.elem_word(s, d.int.expect("matched"));
                self.assign(&Val::Part(*d), Val::Part(w))
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

    /// An element of an array of integers as the integer it is; any other part as it is.
    fn int_elem(&mut self, p: Part) -> Val {
        match p.int {
            Some(e) if p.dims.n == 1 => {
                let r = self.whole(p);
                Val::Int(IntV {
                    r,
                    lo: e.lo,
                    hi: e.hi,
                    down: e.down,
                    place: None,
                    fresh: false,
                    leaf: None,
                })
            }
            _ => Val::Part(p),
        }
    }

    /// The word of integer `s` for an element of an array of integers, checked against the
    /// element's subtype unless it cannot leave it.
    fn elem_word(&mut self, s: IntV, e: IntElem) -> Part {
        if (s.lo < e.lo || s.hi > e.hi)
            && let Some((sub, obj)) = e.place
        {
            let site = self.site(Check::Range {
                ty: subtype_name(self.db, sub),
                lo: e.lo,
                hi: e.hi,
                down: e.down,
                what: obj_desc(self.db, obj),
            });
            self.ops.push(Op::Range {
                a: s.r,
                lo: e.lo,
                hi: e.hi,
                site,
            });
        }
        part_of(s.r, 64)
    }

    /// A dynamic element read into a temporary; any other part as it is.
    fn materialise(&mut self, p: Part) -> Part {
        let Some((idx, b, stride, _)) = p.at else {
            return p;
        };
        let site = self.index_site(&p, b);
        let mut t = self.tmp(p.len);
        (t.dims, t.int) = (p.dims, p.int);
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
            Val::Null(_) => Err("a null array as an operand".into()),
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
                // An element chosen at run time keeps its index for a static selection in it.
                let v = self.name(*prefix, target)?;
                let b = outer(&v).ok_or("an index of a value with no index range")?;
                let ix = match self.static_int(*i) {
                    Ok(i) => Ok(elem_of(b, i).ok_or("an index outside its array's range")?),
                    Err(_) => {
                        let i = self.expr(*i)?;
                        let i = self.int(i)?;
                        // Of an array of null elements alone no operation checks the index:
                        // its subtype must keep it inside.
                        let (lo, hi) = b.low_high();
                        if all_null(&v) && (i.lo < lo || i.hi > hi) {
                            return Err("an index chosen at run time of an array of null \
                                        elements"
                                .into());
                        }
                        Err(i.r)
                    }
                };
                // An element of an element, both chosen at run time, assigned (`a(i)(j) <=`).
                if target
                    && let (Err(rj), Val::Part(p)) = (ix, &v)
                    && p.at.is_some()
                {
                    return self.index_twice(*p, rj, b);
                }
                let v = if ix.is_err() {
                    self.no_at(v, target)?
                } else {
                    v
                };
                at_each(v, b, &mut |mut p| {
                    let ew = p.dims.elem_len();
                    match ix {
                        Ok(el) => {
                            p.lo += el * ew;
                            if let Some(at) = &mut p.at {
                                at.3 += el * ew;
                            }
                        }
                        Err(r) => p.at = Some((r, b, ew, 0)),
                    }
                    p.len = ew;
                    p.dims = p.dims.rest();
                    p
                })
            }
            ExprKind::Slice { prefix, range } => {
                let Ok((sb, _)) = self.static_range(range) else {
                    return self.dynamic_slice(*prefix, range, target);
                };
                let v = self.name(*prefix, target)?;
                let b = outer(&v).ok_or("a slice of a value with no index range")?;
                // A null slice, of any direction and bounds (LRM 8.5): nothing of the prefix.
                if sb.is_empty() {
                    return null_of(v, sb);
                }
                if sb.dir != b.dir {
                    return Err("a slice in the other direction than its array".into());
                }
                let n = sb.len() as u32;
                // The slice's rightmost element is its element 0.
                let lo = elem_of(b, sb.right).ok_or("a slice outside its array's range")?;
                elem_of(b, sb.left).ok_or("a slice outside its array's range")?;
                at_each(v, b, &mut |mut p| {
                    let ew = p.dims.elem_len();
                    p.lo += lo * ew;
                    if let Some(at) = &mut p.at {
                        at.3 += lo * ew;
                    }
                    p.len = n * ew;
                    p.dims = p.dims.with_first(sb);
                    p
                })
            }
            // The prefix of an index or slice that is a value, not a name (`f(x)(7 downto 0)`):
            // read only.
            ExprKind::Call { .. } | ExprKind::Qualified { .. } | ExprKind::Conv { .. }
                if !target =>
            {
                self.expr(x)
            }
            _ => Err("a name that is not supported in cycle mode yet".into()),
        }
    }

    /// Element `j` (of index range `b2`) of part `p` of an element chosen at run time, as one
    /// element at an index of its own over the array's elements of `p`'s element width: element
    /// positions `p1` of the outer array (`k` such elements in each of its elements) and `p2` of
    /// `p` give `p1 * k + p2 + k - len(p)` from the left. Both indices checked first, in order.
    fn index_twice(&mut self, mut p: Part, j: Ref, b2: Bounds) -> R<Val> {
        let (i, b1, s1, off) = p.at.expect("chosen at run time");
        let ew = p.dims.elem_len();
        if ew == 0 || s1 % ew != 0 || !matches!(p.r, Ref::Sig(_) | Ref::Loc(_)) {
            return Err("an index or slice of an element chosen at run time".into());
        }
        let k = i64::from(s1 / ew);
        let mut pos = Vec::new();
        for (x, b) in [(i, b1), (j, b2)] {
            let (lo, hi) = b.low_high();
            let site = self.index_site(&p, b);
            self.ops.push(Op::Range { a: x, lo, hi, site });
            // The position from the left.
            let l = self.int_lit(b.left).r;
            let (a, z) = match b.dir {
                Direction::To => (x, l),
                Direction::Downto => (l, x),
            };
            pos.push(self.int_step(Arith::Sub, a, z));
        }
        let kk = self.int_lit(k).r;
        let t = self.int_step(Arith::Mul, pos[0], kk);
        let t = self.int_step(Arith::Add, t, pos[1]);
        let skew = self.int_lit(k - b2.len() as i64).r;
        let idx = self.int_step(Arith::Add, t, skew);
        let n = b1.len() as i64 * k;
        p.at = Some((
            idx,
            Bounds {
                left: 0,
                dir: Direction::To,
                right: n - 1,
            },
            ew,
            off,
        ));
        p.len = ew;
        p.dims = p.dims.rest();
        Ok(Val::Part(p))
    }

    /// `a op b` on integer words whose result cannot overflow (positions of checked indices).
    fn int_step(&mut self, op: Arith, a: Ref, b: Ref) -> Ref {
        let site = self.site(Check::Arith {
            op,
            ty: "INTEGER".into(),
        });
        let t = self.tmp_int(i64::MIN, i64::MAX);
        self.ops.push(Op::Int {
            op,
            dst: t.r,
            a,
            b,
            lo: i64::MIN,
            hi: i64::MAX,
            site,
        });
        t.r
    }

    /// A slice with bounds chosen at run time and a length known during elaboration (`a(i + 7
    /// downto i)`): the length from the bounds in four trials, each giving every unknown name
    /// a value of its own (one name, one value), the same in all four, and checked at run time.
    fn dynamic_slice(&mut self, prefix: ExprId, range: &RangeC, target: bool) -> R<Val> {
        let unknown = || {
            "a slice whose length is not known during elaboration is not supported in cycle mode \
             yet"
            .to_string()
        };
        let RangeC::Explicit { left, dir, right } = range else {
            return Err(unknown());
        };
        let len = |lx: &Self, t: u64| -> Option<i64> {
            let (l, r) = (lx.probe(*left, t)?, lx.probe(*right, t)?);
            if *dir == Direction::Downto {
                l.checked_sub(r)
            } else {
                r.checked_sub(l)
            }
        };
        let d = len(self, 0).ok_or_else(unknown)?;
        if d < 0 || (1..4).any(|t| len(self, t) != Some(d)) {
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
        p.at = Some((r.r, b, ew, 0));
        p.len = (d as u32 + 1) * ew;
        // Its index range is the bounds' at run time: no index or slice of it is taken.
        p.dims = Dims::NONE;
        Ok(Val::Part(p))
    }

    /// Whether `x` is a slice with a null range known during elaboration.
    fn null_slice(&mut self, x: ExprId) -> bool {
        let ExprKind::Slice { range, .. } = &self.db.exprs[x].kind else {
            return false;
        };
        let (n, l, k) = (self.ops.len(), self.locs, self.known.clone());
        let r = self.static_range(range);
        self.ops.truncate(n);
        self.locs = self.locs.max(l);
        self.known = k;
        matches!(r, Ok((b, _)) if b.is_empty())
    }

    /// The integer value of `x` in trial `t` (the length of a slice chosen at run time): every
    /// integer subexpression elaboration does not know takes a value of its own, the same for
    /// every use of one object (or of one source text), so `hi downto lo` of two variables is
    /// not taken for a slice of one element. The length is checked at run time either way.
    fn probe(&self, x: ExprId, t: u64) -> Option<i64> {
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
                db.fold_with(x, &mut |c| self.probe(c, t).map(ConstValue::Int))
                    .ok()
                    .flatten()?
                    .as_int()
            }
            ExprKind::ImplicitConv { operand } => self.probe(*operand, t),
            ExprKind::Obj(d) => Some(trial_value(d.index() as u64, t)),
            _ => {
                use std::hash::{Hash, Hasher};
                let sp = db.exprs[x].span;
                let f = db.sources.get(sp.file);
                let text = (f.text())
                    .get(f.text_offset(sp.start)..f.text_offset(sp.end))?
                    .split_whitespace()
                    .collect::<String>();
                let mut h = std::collections::hash_map::DefaultHasher::new();
                text.hash(&mut h);
                Some(trial_value(h.finish() | 1 << 63, t))
            }
        }
    }

    fn array_part(&mut self, prefix: ExprId, target: bool) -> R<Part> {
        match self.array_val(prefix, target)? {
            Val::Part(p) => Ok(p),
            _ => Err("an index or slice of a record or an integer".into()),
        }
    }

    /// The array `prefix` names: a logic array, or an array of records (a record of arrays, one
    /// per field, whose outermost index range is the array's).
    fn array_val(&mut self, prefix: ExprId, target: bool) -> R<Val> {
        let v = self.name(prefix, target)?;
        self.no_at(v, target)
    }

    fn no_at(&mut self, v: Val, target: bool) -> R<Val> {
        match v {
            Val::Part(p) if p.at.is_none() => Ok(Val::Part(p)),
            // A part of an element chosen at run time, read: the element first.
            Val::Part(p) if !target => {
                self.note_reads(&Val::Part(p));
                Ok(Val::Part(self.materialise(p)))
            }
            Val::Part(_) => Err("an index or slice of an element chosen at run time".into()),
            Val::Rec(fs) => (fs.into_iter())
                .map(|f| self.no_at(f, target))
                .collect::<R<Vec<_>>>()
                .map(Val::Rec),
            Val::Int(_) => Err("an index or slice of an integer".into()),
            v @ Val::Null(_) => Ok(v),
        }
    }

    fn object(&mut self, x: ExprId, d: DeclId, target: bool) -> R<Val> {
        let (db, e) = (self.db, self.e);
        let decl = &db.decls[d];
        // An alias of an object: its name, with the alias's index range when it has a subtype.
        if let yavs_sem::decl::DeclKind::Alias(yavs_sem::decl::Alias::Object {
            target: t,
            subtype,
        }) = &decl.kind
        {
            let v = self.name(*t, target)?;
            return match (v, subtype) {
                (Val::Part(p), Some(st)) if p.at.is_none() => Ok(Val::Part(self.rebound(p, *st)?)),
                (Val::Part(_), Some(_)) => Err("an alias of an element chosen at run time".into()),
                (v, _) => Ok(v),
            };
        }
        let yavs_sem::decl::DeclKind::Object(o) = &decl.kind else {
            return Err(format!("the name `{}`, which is not an object", decl.name));
        };
        use yavs_sem::decl::ObjClass;
        // A formal or local of a call being inlined.
        match self.bound(d) {
            Some(Bind::Const(_)) if target && !self.bounds => {
                return Err("an assignment to a constant".into());
            }
            Some(Bind::Const(c)) => return self.literal(db.base(o.subtype), &c),
            Some(Bind::Val(v)) => return Ok(v),
            None => {}
        }
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
                let v = self
                    .tree(&so.shape, d, &mut |_| {
                        next += 1;
                        Ref::Loc(next - 1)
                    })
                    .map_err(|w| format!("{w} (signal `{}`)", decl.name))?;
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
                let v = self
                    .tree(shape, d, &mut |_| Ref::Loc(u32::MAX))
                    .map_err(|w| format!("{w} (variable `{}`)", decl.name))?;
                let v = self.place_vars(v, o.default.is_some());
                self.vars.insert(d, v.clone());
                // The initial value, once at initialisation (a combinational process assigns
                // before it reads, so it never sees it): evaluated where the run starts, so the
                // values known at this point of the body are not.
                if let Some(init) = o.default {
                    let known = std::mem::take(&mut self.known);
                    let made = self.capture(|lx| {
                        let s = lx.expr_h(init, Some(&v))?;
                        lx.assign(&v, s)
                    });
                    self.known = known;
                    self.init_ops.extend(made?);
                }
                Ok(v)
            }
            ObjClass::SharedVariable => self.shared_var(d),
            ObjClass::Constant if target && !self.bounds => {
                Err("an assignment to a constant".into())
            }
            ObjClass::Constant => {
                if let Some(c) = self.consts.get(&d).cloned() {
                    let mut v = self.literal(db.base(o.subtype), &c)?;
                    if let Val::Int(i) = &mut v
                        && let Some(b) = int_bounds(db, o.subtype)
                    {
                        (i.lo, i.hi) = b;
                    }
                    return Ok(v);
                }
                match self.fold(x) {
                    // Named in the messages of index checks, as the normal mode names it.
                    Some(c) => Ok(named(self.literal(db.base(o.subtype), &c)?, d)),
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

    /// A shared variable of an architecture or block (not of a protected type) that this process
    /// alone uses (`region::shared_rule`): a variable of the process, kept from one edge to the
    /// next, read and written in place in program order, starting from the value elaboration gave
    /// it (`ElabConfig::keep_values`). Named in a subprogram it stays out.
    fn shared_var(&mut self, d: DeclId) -> R<Val> {
        if let Some(v) = self.vars.get(&d) {
            return Ok(v.clone());
        }
        let (db, e) = (self.db, self.e);
        let decl = &db.decls[d];
        if !self.frames.is_empty() {
            return Err(format!(
                "the shared variable `{}` in a subprogram",
                decl.name
            ));
        }
        let yavs_sem::decl::DeclKind::Object(o) = &decl.kind else {
            return Err("a shared variable that is not an object".into());
        };
        if matches!(
            db.types[db.base(o.subtype)].kind,
            TypeKind::Protected { .. }
        ) {
            return Err("a shared variable of a protected type".into());
        }
        // The elaborated region that declares it, from the process outwards.
        let mut s = Some(self.p.scope);
        let region = loop {
            let Some(x) = s else {
                return Err(format!(
                    "the shared variable `{}`, which is not of an architecture or block around \
                     the process",
                    decl.name
                ));
            };
            if e.regions.get(&x).is_some_and(|r| r.contains(&decl.scope)) {
                break x;
            }
            s = e.hier.scope(x).parent;
        };
        let shape = (e.var_shapes.get(&(Some(region), d))).ok_or_else(|| {
            format!(
                "the shared variable `{}`, whose shape elaboration did not compute",
                decl.name
            )
        })?;
        let init = (e.region_vals.get(&(Some(region), d)))
            .ok_or_else(|| {
                format!(
                    "the shared variable `{}`, whose initial value elaboration did not compute",
                    decl.name
                )
            })?
            .clone();
        let v = self
            .tree(shape, d, &mut |_| Ref::Loc(u32::MAX))
            .map_err(|w| format!("{w} (shared variable `{}`)", decl.name))?;
        let v = self.place_vars(v, true);
        self.vars.insert(d, v.clone());
        let known = std::mem::take(&mut self.known);
        let made = self.capture(|lx| {
            let s = lx.literal(db.base(o.subtype), &init)?;
            lx.assign(&v, s)
        });
        self.known = known;
        self.init_ops.extend(made?);
        Ok(v)
    }

    /// Give a variable's leaves their slots: a logic leaf starts at 0 (its `'U'`, counted, unless
    /// it has an initial value), an integer at its `'left`.
    fn place_vars(&mut self, v: Val, has_init: bool) -> Val {
        match v {
            Val::Part(mut p) => {
                p.r = self.loc(words(p.full));
                if let Ref::Loc(w) = p.r {
                    self.var_words.push((w, words(p.full)));
                    match p.int {
                        // Integer elements start at their `'left`.
                        Some(e) if !has_init => {
                            let left = if e.down { e.hi } else { e.lo };
                            if left != 0 {
                                (0..p.full / 64).for_each(|k| self.init.push((w + k, left as u64)));
                            }
                        }
                        Some(_) => {}
                        None if !has_init => self.uninit += u64::from(p.full),
                        None => {}
                    }
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
            v @ Val::Null(_) => v,
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
            // An array of integers or enumerations: its kernel signal of its own, a word per
            // element in the image, from the right as every array of the lowering (the kernel
            // keeps the elements of a byte signal from the left: `rt.rs` turns them round).
            Val::Part(mut p) if p.int.is_some() => {
                let l = leaf_of(p.r)?;
                let leaf = nets.leaf(l);
                let part = so.part((l.0 - so.leaves.0.0) as usize);
                let (off, len) = part.map_or((0, leaf.ty.len), |r| (r.offset, r.len));
                let at = nets.loc[l.index()];
                if 64 * len != p.len
                    || !matches!(leaf.ty.elem, yavs_kernel::Elem::Bytes(1 | 4 | 8))
                    || at.base != 0
                    || nets.signal_len(at.signal) != leaf.ty.len
                    || leaf.implicit.is_some()
                {
                    return Err(format!(
                        "the signal {}, whose kernel form differs",
                        self.e.leaf_name(l)
                    ));
                }
                p.r = Ref::Sig(at.signal.0);
                p.lo = 64 * (leaf.ty.len - off - len);
                p.full = 64 * leaf.ty.len;
                p.leaf = Some(l);
                Ok(Val::Part(p))
            }
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
                // A scalar port collapsed onto one element of an array of integers: that element,
                // a word of the array's (from the right, as `Val::Part` above).
                if let Some(r) = so.part((l.0 - so.leaves.0.0) as usize)
                    && r.len == 1
                    && matches!(leaf.ty.elem, yavs_kernel::Elem::Bytes(1 | 4 | 8))
                    && at.base == 0
                    && nets.signal_len(at.signal) == leaf.ty.len
                    && leaf.implicit.is_none()
                {
                    return Ok(Val::Part(Part {
                        r: Ref::Sig(at.signal.0),
                        lo: 64 * (leaf.ty.len - r.offset - 1),
                        len: 64,
                        full: 64 * leaf.ty.len,
                        dims: Dims::wrap(WORD, Dims::NONE).expect("one deep"),
                        fresh: false,
                        leaf: Some(l),
                        at: None,
                        obj: None,
                        lit: None,
                        int: Some(IntElem {
                            lo: i.lo,
                            hi: i.hi,
                            down: i.down,
                            place: i.place,
                        }),
                    }));
                }
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
            v @ Val::Null(_) => Ok(v),
        }
    }

    /// Count the signal elements a value read (per leaf, from its element 0).
    fn note_reads(&mut self, v: &Val) {
        let nets = &self.e.nets;
        match v {
            Val::Part(p) => {
                if let Some(l) = p.leaf {
                    let base = nets.loc[l.index()].base;
                    // An element chosen at run time: the whole array it is of.
                    let (lo, n) = match p.at {
                        Some((_, b, stride, off)) => (p.lo - off, stride * b.len() as u32),
                        None => (p.lo, p.len),
                    };
                    self.reads.insert(l);
                    // An array of integers: a word per element, from the right; the kernel's
                    // elements (the subset checker's) from the left.
                    let (lo, n) = match p.int {
                        Some(_) => (p.full / 64 - lo / 64 - n / 64, n / 64),
                        None => (lo, n),
                    };
                    self.rparts.push((l, lo - base, n));
                }
            }
            Val::Int(IntV { leaf: Some(l), .. }) => {
                self.reads.insert(*l);
                self.rparts.push((*l, 0, 1));
            }
            Val::Int(_) | Val::Null(_) => {}
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
                    int: None,
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
            Shape::Array { .. } if let Some((ds, el)) = self.int_dims(shape)? => {
                self.int_part(ds, el, d, leaf)
            }
            Shape::Array { dims, elem, .. } if matches!(**elem, Shape::Record { .. }) => {
                let [b] = &dims[..] else {
                    return Err(
                        "a multi-dimensional array is not supported in cycle mode yet".into(),
                    );
                };
                self.fields_of(elem, *b, d, leaf)
            }
            Shape::Array { .. } => {
                let ds = self.logic_dims(shape)?;
                let n = u64::from(ds.elem_len()) * ds.b[0].len();
                if n == 0 {
                    // Its leaf, of no elements, still counts.
                    leaf(0);
                    return Ok(Val::Null(ds));
                }
                if n > VEC_MAX {
                    return Err(format!(
                        "a vector of {n} elements (cycle mode takes 1 to 2**26 for now)"
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
                    int: None,
                }))
            }
            Shape::Record { fields, .. } => fields
                .iter()
                .map(|(_, f)| self.tree(f, d, leaf))
                .collect::<R<Vec<_>>>()
                .map(Val::Rec),
        }
    }

    /// The leaves of an array of records of index range `outer` and element shape `shape`, struct
    /// of arrays as the kernel keeps them: each field an array over the elements.
    fn fields_of(
        &self,
        shape: &Shape,
        outer: Bounds,
        d: DeclId,
        leaf: &mut dyn FnMut(u32) -> Ref,
    ) -> R<Val> {
        if let Shape::Record { fields, .. } = shape {
            return (fields.iter())
                .map(|(_, f)| self.fields_of(f, outer, d, leaf))
                .collect::<R<Vec<_>>>()
                .map(Val::Rec);
        }
        if let Some((ds, el)) = self.int_dims(shape)? {
            let ds = Dims::wrap(outer, ds)
                .ok_or_else(|| "an array nested more than four deep".to_string())?;
            return self.int_part(ds, el, d, leaf);
        }
        if let Shape::Scalar { elem, .. } = shape
            && *elem != yavs_kernel::Elem::Logic
        {
            let ds = Dims::wrap(outer, Dims::wrap(WORD, Dims::NONE).expect("one deep"))
                .expect("two deep");
            return self.int_part(ds, shape, d, leaf);
        }
        let ds = Dims::wrap(outer, self.logic_dims(shape)?)
            .ok_or_else(|| "an array nested more than four deep".to_string())?;
        let n = u64::from(ds.elem_len()) * ds.b[0].len();
        if n == 0 {
            leaf(0);
            return Ok(Val::Null(ds));
        }
        if n > VEC_MAX {
            return Err(format!(
                "a vector of {n} elements (cycle mode takes 1 to 2**26 for now)"
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
            int: None,
        }))
    }

    /// The index ranges of an array of integers or enumerations (or of such arrays), the word of
    /// each element innermost, and the element's shape; `None` for an array of logic elements.
    fn int_dims<'s>(&self, shape: &'s Shape) -> R<Option<(Dims, &'s Shape)>> {
        let Shape::Array { dims, elem, .. } = shape else {
            return Ok(None);
        };
        let inner = match &**elem {
            Shape::Scalar { elem: e, .. } if *e != yavs_kernel::Elem::Logic => {
                Some((Dims::wrap(WORD, Dims::NONE).expect("one deep"), &**elem))
            }
            Shape::Array { .. } => self.int_dims(elem)?,
            _ => None,
        };
        let Some((inner, el)) = inner else {
            return Ok(None);
        };
        let [b] = &dims[..] else {
            return Err("a multi-dimensional array is not supported in cycle mode yet".into());
        };
        let ds = Dims::wrap(*b, inner)
            .ok_or_else(|| "an array nested more than four deep".to_string())?;
        Ok(Some((ds, el)))
    }

    /// The place of an array of integers or enumerations of index ranges `ds` (the word last) and
    /// element shape `el`: a word per element.
    fn int_part(
        &self,
        ds: Dims,
        el: &Shape,
        d: DeclId,
        leaf: &mut dyn FnMut(u32) -> Ref,
    ) -> R<Val> {
        let Shape::Scalar {
            ty,
            sub,
            range: Some(r),
            ..
        } = el
        else {
            return Err(
                "an array of real or physical values is not supported in cycle mode yet".into(),
            );
        };
        if !matches!(
            self.db.types[*ty].kind,
            TypeKind::Integer | TypeKind::Enum { .. }
        ) {
            return Err(
                "an array of real or physical values is not supported in cycle mode yet".into(),
            );
        }
        let n = (ds.b[..ds.n as usize - 1].iter()).fold(1u64, |a, b| a.saturating_mul(b.len()));
        if n == 0 || n > 1 << 14 {
            return Err(format!(
                "an array of {n} integers (cycle mode takes 1 to 2**14 for now)"
            ));
        }
        let (lo, hi) = r.low_high();
        let n = 64 * n as u32;
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
            int: Some(IntElem {
                lo,
                hi,
                down: r.dir == Direction::Downto,
                place: Some((*sub, d)),
            }),
        }))
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
        // A globally static call elaboration evaluated for this process (outside an inlined
        // call, whose formals the call's arguments may name).
        // Its checks stay: a value or an argument outside its subtype leaves the call to the
        // lowering, which stops the run as the normal mode does.
        if self.frames.is_empty()
            && let Some(v) = self.e.region_calls.get(&(self.p.scope, x))
            && let ExprKind::Call { callee, args, .. } = &db.exprs[x].kind
            && self.call_fits(*callee, args, v)
        {
            return Some(v.clone());
        }
        match &db.exprs[x].kind {
            ExprKind::Obj(d) => {
                if let Some(b) = self.bound(*d) {
                    return match b {
                        Bind::Const(c) => Some(c),
                        Bind::Val(v) => self.known_val(&v),
                    };
                }
                if let Some(c) = self.consts.get(d) {
                    return Some(c.clone());
                }
                match &db.decls[*d].kind {
                    yavs_sem::decl::DeclKind::Object(o)
                        if o.class == yavs_sem::decl::ObjClass::Variable =>
                    {
                        self.known_val(self.vars.get(d)?)
                    }
                    yavs_sem::decl::DeclKind::Object(o)
                        if o.class == yavs_sem::decl::ObjClass::Constant =>
                    {
                        // A constant of the regions around the process, else of a package.
                        (self.e.constant(self.p.scope, *d))
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
            // `x'length` and its friends of an array object whose range is fixed: a formal or
            // local of an inlined call, a signal, a variable of the process.
            ExprKind::Attr { attr, prefix, args }
                if args.is_empty()
                    && let Some(b) = match prefix {
                        AttrPrefix::Expr(p) => match db.exprs[*p].kind {
                            ExprKind::Obj(d) => self.obj_bounds(d),
                            _ => None,
                        },
                        AttrPrefix::Subtype(st) => self.subtype_fold(*st),
                        _ => None,
                    } =>
            {
                let (lo, hi) = b.low_high();
                Some(ConstValue::Int(match attr {
                    PredefAttr::Length => b.len() as i64,
                    PredefAttr::Left => b.left,
                    PredefAttr::Right => b.right,
                    PredefAttr::Low => lo,
                    PredefAttr::High => hi,
                    _ => return None,
                }))
            }
            // A pure function of the design on constants: run by the analysis's evaluator.
            ExprKind::Call {
                callee,
                args,
                method_of: None,
                ..
            } if (db.decls[*callee].kind.subprogram()).is_some_and(|sp| {
                sp.kind == yavs_sem::decl::SubprogramKind::Function { pure: true }
                    && matches!(sp.body, yavs_sem::decl::Body::Hir { .. })
                    && !["ieee.", "std."]
                        .iter()
                        .any(|p| sp.mangled.as_str().starts_with(p))
            }) =>
            {
                let mut known = self.consts.clone();
                for f in &self.frames {
                    for (d, b) in &f.binds {
                        if let Bind::Const(c) = b {
                            known.insert(*d, c.clone());
                        }
                    }
                }
                let v = crate::check::const_value(db, self.e, self.p, known, x)?;
                // The checks of the call stay: a value or an argument outside its subtype is
                // left to the inlined call, which stops the run as the normal mode does.
                self.call_fits(*callee, args, &v).then_some(v)
            }
            // A field of a record constant (`registers(i).width` of a generic table).
            ExprKind::Field { prefix, elem } => match self.fold(*prefix)? {
                ConstValue::Record(fs) => fs.get(*elem as usize).cloned(),
                _ => None,
            },
            // An element of a constant at indices known here (`grid(r, c)` in an unrolled loop).
            ExprKind::Index { prefix, indices } => {
                let Some(ConstValue::Array(a)) = self.fold(*prefix) else {
                    return None;
                };
                let ix: Option<Vec<i64>> = (indices.iter())
                    .map(|&i| self.fold(i).and_then(|c| c.as_int()))
                    .collect();
                crate::check::const_elem(&a, &ix?)
            }
            _ => db.fold_with(x, &mut |c| self.fold(c)).ok().flatten(),
        }
    }

    /// Whether value `v` of a call of `callee` and the call's arguments, all known here, lie in
    /// their subtypes (integers and enumerations; another value only of an unconstrained one).
    fn call_fits(&self, callee: DeclId, args: &[yavs_sem::hir::Arg], v: &ConstValue) -> bool {
        let db = self.db;
        let fits = |st: SubtypeId, v: &ConstValue| match (int_bounds(db, st), v.as_int()) {
            (Some((lo, hi)), Some(v)) => (lo..=hi).contains(&v),
            (Some(_), None) => false,
            (None, _) => matches!(
                db.subtypes[st].constraint,
                Constraint::None | Constraint::Array { dims: None, .. }
            ),
        };
        let Some(sp) = db.decls[callee].kind.subprogram() else {
            return false;
        };
        sp.ret.is_some_and(|r| fits(r, v))
            && args.iter().all(|a| {
                let Actual::Expr(ax) = a.actual else {
                    return false;
                };
                (sp.params.get(usize::from(a.formal)))
                    .zip(self.fold(ax))
                    .is_some_and(|(p, c)| fits(p.subtype, &c))
            })
    }

    /// The range of a scalar subtype or the index range of a one-dimensional array subtype when
    /// it is known here, without lowering anything (a range of generics elaboration evaluated).
    fn subtype_fold(&self, st: SubtypeId) -> Option<Bounds> {
        let db = self.db;
        let r = match &db.subtypes[st].constraint {
            Constraint::Range(r) => r,
            Constraint::Array { dims: Some(d), .. } if d.len() == 1 => &d[0],
            _ => {
                let (l, dir, r) = db.static_bounds(st)?;
                return Some(Bounds {
                    left: l.as_int()?,
                    dir,
                    right: r.as_int()?,
                });
            }
        };
        let RangeC::Explicit { left, dir, right } = r else {
            return None;
        };
        match (self.fold(*left), self.fold(*right)) {
            (Some(l), Some(r)) => Some(Bounds {
                left: l.as_int()?,
                dir: *dir,
                right: r.as_int()?,
            }),
            _ => self.e.range(self.p.scope, *left),
        }
    }

    /// The index range of array object `d` when it is fixed, without lowering anything.
    fn obj_bounds(&self, d: DeclId) -> Option<Bounds> {
        if let Some(b) = self.bound(d) {
            return match b {
                Bind::Val(Val::Part(v)) if v.dims.n > 0 && v.at.is_none() => Some(v.dims.b[0]),
                _ => None,
            };
        }
        let shape = match &self.db.decls[d].kind {
            yavs_sem::decl::DeclKind::Object(o) => match o.class {
                yavs_sem::decl::ObjClass::Signal => {
                    &self.e.nets.obj(self.e.signal(self.p.scope, d)?).shape
                }
                yavs_sem::decl::ObjClass::Variable => {
                    let up = (self.e.hier.scope(self.p.scope).parent, d);
                    (self.e.var_shapes.get(&(Some(self.p.scope), d)))
                        .or_else(|| self.e.var_shapes.get(&up))?
                }
                _ => return None,
            },
            _ => return None,
        };
        match shape {
            Shape::Array { dims, .. } => dims.first().copied(),
            _ => None,
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
        if let Some(h) = hint
            && all_null(h)
            && self.const_agg(x)
        {
            return Ok(self.fresh_like(h));
        }
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
                    Val::Part(p) => {
                        let p = self.materialise(p);
                        Ok(self.int_elem(p))
                    }
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
            ExprKind::Attr {
                attr: attr @ (PredefAttr::Pos | PredefAttr::Val),
                prefix: AttrPrefix::Subtype(st),
                args,
            } if args.len() == 1 => self.pos_val(*attr == PredefAttr::Pos, *st, args[0]),
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

    /// Whether `x` is an aggregate whose element values, nested aggregates' too, elaboration
    /// knows (nothing in it can fail).
    fn const_agg(&self, x: ExprId) -> bool {
        let ExprKind::Aggregate { assocs, .. } = &self.db.exprs[x].kind else {
            return false;
        };
        (assocs.iter()).all(|a| {
            (a.choices.iter()).all(|c| match c {
                yavs_sem::hir::Choice::Expr(e) => self.fold(*e).is_some(),
                yavs_sem::hir::Choice::Range(_) => false,
                _ => true,
            }) && (self.fold(a.value).is_some() || self.const_agg(a.value))
        })
    }

    /// `T'pos(a)` (`pos`) or `T'val(a)` of a discrete type: an enumeration's position is its word
    /// (a logic value's 0 or 1 is the position of `'0'` or `'1'`), an integer is itself. `'val`
    /// checks the position against the subtype; of a logic type it takes only `'0'` and `'1'`.
    fn pos_val(&mut self, pos: bool, st: SubtypeId, a: ExprId) -> R<Val> {
        let db = self.db;
        let t = db.base(st);
        let logic = yavs_elab::elem_of(db, t) == yavs_kernel::Elem::Logic;
        let lits = match &db.types[t].kind {
            TypeKind::Enum { lits } => Some(lits.len() as i64),
            TypeKind::Integer => None,
            _ => return Err("'pos or 'val of a type that is not discrete".into()),
        };
        // The position of `'0'` in a logic type: 0 in BIT and BOOLEAN, 2 in STD_ULOGIC.
        let zero = match lits {
            Some(9) if logic => 2,
            Some(2) if logic => 0,
            Some(_) if logic => return Err("'pos or 'val of a logic type of another form".into()),
            _ => 0,
        };
        let v = self.expr(a)?;
        if pos {
            return match v {
                Val::Int(i) => Ok(Val::Int(IntV {
                    place: None,
                    leaf: None,
                    ..i
                })),
                Val::Part(p) if logic && p.len == 1 => {
                    let c = self.whole(p);
                    let (one, z) = (self.int_lit(zero + 1), self.int_lit(zero));
                    let out = self.tmp_int(zero, zero + 1);
                    self.ops.push(Op::Select {
                        dst: out.r,
                        c,
                        a: one.r,
                        b: z.r,
                        len: 64,
                    });
                    Ok(Val::Int(out))
                }
                _ => Err("'pos of a value of another form".into()),
            };
        }
        let i = self.int(v)?;
        match lits {
            // An integer type: the value, checked against the subtype.
            None => {
                let (lo, hi) =
                    int_bounds(db, st).ok_or("'val of an integer type without bounds")?;
                if i.lo < lo || i.hi > hi {
                    let site = self.site(Check::Range {
                        ty: subtype_name(db, st),
                        lo,
                        hi,
                        down: false,
                        what: String::new(),
                    });
                    self.ops.push(Op::Range {
                        a: i.r,
                        lo,
                        hi,
                        site,
                    });
                }
                Ok(Val::Int(IntV {
                    lo: i.lo.max(lo),
                    hi: i.hi.min(hi),
                    place: None,
                    leaf: None,
                    ..i
                }))
            }
            Some(_) if logic => {
                if i.lo < zero || i.hi > zero + 1 {
                    return Err("'val of a logic type at a position that may be a metavalue".into());
                }
                let one = self.int_lit(zero + 1);
                let t = self.tmp(1);
                self.ops.push(Op::IntCmp {
                    op: Cmp::Eq,
                    dst: t.r,
                    a: i.r,
                    b: one.r,
                });
                Ok(Val::Part(t))
            }
            // An enumeration: the position, checked against the subtype's (`value 3 outside of
            // E_T range 0 to 2`, the normal mode's text).
            Some(_) => {
                let (lo, hi) = int_bounds(db, st).ok_or("'val of an enumeration without bounds")?;
                if i.lo < lo || i.hi > hi {
                    let site = self.site(Check::Range {
                        ty: subtype_name(db, st),
                        lo,
                        hi,
                        down: false,
                        what: String::new(),
                    });
                    self.ops.push(Op::Range {
                        a: i.r,
                        lo,
                        hi,
                        site,
                    });
                }
                Ok(Val::Int(IntV {
                    lo: i.lo.max(lo),
                    hi: i.hi.min(hi),
                    place: None,
                    leaf: None,
                    ..i
                }))
            }
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
                // A constrained target gives the value its index ranges (LRM 9.3.6).
                if !open {
                    return Ok(Val::Part(self.rebound(p, to)?));
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
            Val::Null(ds) => self.rebound_null(ds, to),
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
        let int = match hint {
            Some(Val::Part(p)) => p.int,
            _ => None,
        };
        let (n, dims) = match hint {
            Some(Val::Part(p)) if p.dims.n > 0 => (p.len, p.dims),
            _ => {
                let st = db.exprs[x].subtype;
                let TypeKind::Array { elem, indexes } = &db.types[db.base(st)].kind else {
                    return Err("an array aggregate of something that is not an array".into());
                };
                let b = match (self.subtype_bounds(st), bounds, &indexes[..]) {
                    (Ok(b), ..) => b,
                    // Named association: the range of the choices, in the direction of the index
                    // subtype (LRM 9.3.3.3).
                    (Err(w), AggBounds::Named { .. }, [ix]) => {
                        let dir = self.subtype_bounds(*ix)?.dir;
                        let (lo, hi) = self.choice_span(assocs)?.ok_or(w)?;
                        match dir {
                            Direction::To => Bounds {
                                left: lo,
                                dir,
                                right: hi,
                            },
                            Direction::Downto => Bounds {
                                left: hi,
                                dir,
                                right: lo,
                            },
                        }
                    }
                    (Err(w), ..) => return Err(w),
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
        let elem_part = Part {
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
            int: None,
        };
        let elem_hint = Val::Part(elem_part);
        let count = b.len() as usize;
        let mut done = vec![false; count];
        // Element positions from the left, with their values.
        let mut put: Vec<(Vec<usize>, Part)> = Vec::new();
        // Slice pieces (`39 downto 0 => v`): the rightmost position and the value.
        let mut pieces: Vec<(usize, Part)> = Vec::new();
        for (k, a) in assocs.iter().enumerate() {
            if a.slice_piece {
                let [Choice::Range(r)] = &a.choices[..] else {
                    return Err(
                        "an aggregate with slice pieces without a range choice is not \
                                supported in cycle mode yet"
                            .into(),
                    );
                };
                let (rb, _) = self.static_range(r)?;
                if rb.is_empty() {
                    continue;
                }
                let (Some(pl), Some(pr)) = (b.pos(rb.left), b.pos(rb.right)) else {
                    return Err("an aggregate choice outside the range".into());
                };
                if rb.dir != b.dir {
                    return Err("a slice piece in the other direction than its aggregate".into());
                }
                let (pl, pr) = (pl as usize, pr as usize);
                if done[pl..=pr].iter().any(|&d| d) {
                    return Err("an aggregate with an element given twice".into());
                }
                done[pl..=pr].fill(true);
                let m = (pr - pl + 1) as u32;
                let hint = Val::Part(Part {
                    len: m * ew,
                    full: m * ew,
                    dims: dims.with_first(rb),
                    ..elem_part
                });
                let v = self.expr_h(a.value, Some(&hint))?;
                let v = self.part(v)?;
                if v.len != m * ew || v.int.is_some() != int.is_some() {
                    return Err("a slice piece of another length".into());
                }
                pieces.push((pr, v));
                continue;
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
            let v = match (self.expr_h(a.value, Some(&elem_hint))?, int) {
                (Val::Int(i), Some(e)) => self.elem_word(i, e),
                (v, _) => self.part(v)?,
            };
            if v.len != ew {
                return Err("an aggregate element of another length".into());
            }
            put.push((at, v));
        }
        if done.iter().any(|d| !d) {
            return Err("an aggregate that misses an element".into());
        }
        let mut t = self.tmp(n);
        (t.dims, t.int) = (dims, int);
        // One value for every element of an array of scalars: spread it.
        if let [(at, v)] = &put[..]
            && pieces.is_empty()
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
        for (pr, v) in pieces {
            let len = v.len;
            let s = self.whole(v);
            self.ops.push(Op::Insert {
                dst: t.r,
                src: s,
                lo: (count - 1 - pr) as u32 * ew,
                len,
            });
        }
        Ok(Val::Part(t))
    }

    /// The lowest and highest choice of a named array aggregate, all known during elaboration;
    /// `None` with `others`.
    fn choice_span(&mut self, assocs: &[yavs_sem::hir::AggAssoc]) -> R<Option<(i64, i64)>> {
        let mut span: Option<(i64, i64)> = None;
        for a in assocs {
            for c in a.choices.iter() {
                let (lo, hi) = match c {
                    Choice::Expr(i) => {
                        let i = self.static_int(*i)?;
                        (i, i)
                    }
                    Choice::Range(r) => self.static_range(r)?.0.low_high(),
                    _ => return Ok(None),
                };
                if lo > hi {
                    continue;
                }
                span = Some(span.map_or((lo, hi), |(a, b)| (a.min(lo), b.max(hi))));
            }
        }
        Ok(span)
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
            (ConstValue::Array(a), TypeKind::Array { elem, .. })
                if yavs_elab::elem_of(db, db.base(*elem)) != yavs_kernel::Elem::Logic
                    && matches!(
                        db.types[db.base(*elem)].kind,
                        TypeKind::Integer | TypeKind::Enum { .. }
                    ) =>
            {
                let [(l, d, r)] = a.bounds[..] else {
                    return Err("a multi-dimensional literal".into());
                };
                let b = Bounds {
                    left: l,
                    dir: d,
                    right: r,
                };
                let mut vs: Vec<i64> = Vec::new();
                match &a.elems {
                    Elems::Bytes(bs) => vs.extend(bs.iter().map(|&x| i64::from(x))),
                    Elems::Runs(runs) => {
                        for (n, v) in runs.iter() {
                            let v = v.as_int().ok_or("a literal of an unexpected shape")?;
                            vs.extend(std::iter::repeat_n(v, *n as usize));
                        }
                    }
                }
                if vs.is_empty() || vs.len() as u64 != b.len() || vs.len() > 1 << 14 {
                    return Err("a literal of an unexpected shape".into());
                }
                // Element `p` from the left is word `n - 1 - p`.
                let val: Box<[u64]> = vs.iter().rev().map(|&v| v as u64).collect();
                let mut t = self.tmp(64 * vs.len() as u32);
                self.ops.push(Op::Lit { dst: t.r, val });
                t.dims = Dims::wrap(b, Dims::wrap(WORD, Dims::NONE).expect("one deep"))
                    .expect("two deep");
                t.int = Some(IntElem {
                    lo: *vs.iter().min().expect("not empty"),
                    hi: *vs.iter().max().expect("not empty"),
                    down: false,
                    place: None,
                });
                return Ok(Val::Part(t));
            }
            (ConstValue::Record(fs), TypeKind::Record { elems }) if fs.len() == elems.len() => {
                return (fs.iter().zip(elems.iter()))
                    .map(|(f, el)| self.literal(db.base(el.subtype), f))
                    .collect::<R<Vec<_>>>()
                    .map(Val::Rec);
            }
            // An array of records: a record of arrays, one per field, as the kernel keeps it.
            (ConstValue::Array(a), TypeKind::Array { elem, .. })
                if matches!(db.types[db.base(*elem)].kind, TypeKind::Record { .. }) =>
            {
                let [(l, d, r)] = a.bounds[..] else {
                    return Err("a multi-dimensional literal".into());
                };
                let b = Bounds {
                    left: l,
                    dir: d,
                    right: r,
                };
                let Elems::Runs(runs) = &a.elems else {
                    return Err("a literal of an unexpected shape".into());
                };
                let vals: Vec<&ConstValue> = (runs.iter())
                    .flat_map(|(n, v)| std::iter::repeat_n(v, *n as usize))
                    .collect();
                if vals.is_empty() || vals.len() as u64 != b.len() {
                    return Err("a literal of an unexpected shape".into());
                }
                return self.rec_array_lit(b, db.base(*elem), &vals);
            }
            _ => {}
        }
        let (codes, dims) = self.codes(t, c)?;
        self.logic_lit(codes, dims)
    }

    /// The fields of an array of records of index range `b` whose elements, from the left, are
    /// `vals` (records of type `t`), each as an array over the elements.
    fn rec_array_lit(&mut self, b: Bounds, t: TypeId, vals: &[&ConstValue]) -> R<Val> {
        let db = self.db;
        let TypeKind::Record { elems } = &db.types[t].kind else {
            return Err("a literal of an unexpected shape".into());
        };
        let mut fields = Vec::new();
        for (j, el) in elems.iter().enumerate() {
            let fv: Vec<&ConstValue> = (vals.iter())
                .map(|v| match v {
                    ConstValue::Record(fs) => fs.get(j).ok_or("a literal of an unexpected shape"),
                    _ => Err("a literal of an unexpected shape"),
                })
                .collect::<Result<_, _>>()?;
            let ft = db.base(el.subtype);
            let logic = match &db.types[ft].kind {
                TypeKind::Record { .. } => {
                    fields.push(self.rec_array_lit(b, ft, &fv)?);
                    continue;
                }
                TypeKind::Integer => false,
                TypeKind::Enum { .. } => yavs_elab::elem_of(db, ft) == yavs_kernel::Elem::Logic,
                _ => true,
            };
            if !logic {
                // A word per element, the rightmost first.
                let vs: Vec<i64> = (fv.iter())
                    .map(|v| v.as_int().ok_or("a literal of an unexpected shape"))
                    .collect::<Result<_, _>>()?;
                let val: Box<[u64]> = vs.iter().rev().map(|&v| v as u64).collect();
                let mut p = self.tmp(64 * vs.len() as u32);
                self.ops.push(Op::Lit { dst: p.r, val });
                p.dims = Dims::wrap(b, Dims::wrap(WORD, Dims::NONE).expect("one deep"))
                    .expect("two deep");
                p.int = Some(IntElem {
                    lo: *vs.iter().min().expect("not empty"),
                    hi: *vs.iter().max().expect("not empty"),
                    down: false,
                    place: None,
                });
                fields.push(Val::Part(p));
                continue;
            }
            let mut codes = Vec::new();
            let mut inner = Dims::NONE;
            for v in &fv {
                let (cs, ib) = self.codes(ft, v)?;
                inner = ib;
                codes.extend(cs);
            }
            let ds = Dims::wrap(b, inner).ok_or("a literal nested more than four deep")?;
            if codes.len() as u64 != b.len() * u64::from(ds.elem_len()) {
                return Err("a literal of an unexpected shape".into());
            }
            fields.push(self.logic_lit(codes, ds)?);
        }
        Ok(Val::Rec(fields))
    }

    /// A logic literal of `codes` (row-major, from the left) and index ranges `dims`.
    fn logic_lit(&mut self, codes: Vec<u8>, dims: Dims) -> R<Val> {
        let n = codes.len() as u32;
        if n == 0 {
            return Ok(Val::Null(dims));
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
        for v in [&a, &b] {
            if let Val::Part(Part { lit: Some(_), .. }) = v {
                return Err(META_CMP.into());
            }
        }
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
        if let yavs_sem::decl::DeclKind::Alias(yavs_sem::decl::Alias::NonObject { target }) =
            &decl.kind
        {
            return self.call(x, *target, args);
        }
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
            // The second operand takes the shape of the first (an aggregate), unless it has
            // named choices and no `others`: an operand is no context, so such an aggregate has
            // its own range, in the direction of its index subtype (LRM 9.3.3.3).
            let Actual::Expr(bx) = b.actual else {
                return Err("a call with a default or converted actual".into());
            };
            let named = matches!(&lx.db.exprs[bx].kind, ExprKind::Aggregate { assocs, .. }
                if assocs.iter().any(|a| !a.choices.is_empty())
                    && !assocs.iter().any(|a| a.choices.iter().any(|c| matches!(c, Choice::Others))));
            let b = if named {
                lx.expr(bx)?
            } else {
                lx.expr_h(bx, Some(&a))?
            };
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
                    // The left operand's index range (LRM 9.2.2).
                    let r = outer(&a);
                    return self.gate(g, a, b).map(|v| with_range(v, r));
                }
                match fam {
                    Family::Not => {
                        let v = one(self)?;
                        let r = outer(&v);
                        self.not(v).map(|v| with_range(v, r))
                    }
                    Family::Cat => {
                        // A null operand (`"" & x`): the other one, as an array.
                        let null = |lx: &Self, a: &yavs_sem::hir::Arg| match a.actual {
                            Actual::Expr(x) => matches!(lx.fold(x), Some(ConstValue::Array(c))
                            if c.bounds.len() == 1 && {
                                let (l, d, r) = c.bounds[0];
                                Bounds { left: l, dir: d, right: r }.is_empty()
                            }),
                            _ => false,
                        };
                        if let [a, b] = args
                            && (null(self, a) || null(self, b))
                        {
                            let other = if null(self, a) { b } else { a };
                            let v = self.arg(other)?;
                            return Ok(Val::Part(Part {
                                dims: Dims::NONE,
                                ..self.part(v)?
                            }));
                        }
                        let (a, b) = two(self)?;
                        let (a, b) = match (a, b) {
                            (a @ Val::Null(_), Val::Null(_)) => return Ok(a),
                            (Val::Null(_), v) | (v, Val::Null(_)) => {
                                return Ok(Val::Part(Part {
                                    dims: Dims::NONE,
                                    ..self.part(v)?
                                }));
                            }
                            (a, b) => (self.part(a)?, self.part(b)?),
                        };
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
                    // Records: equal when every element is (LRM 9.2.3).
                    Family::Eq | Family::Neq
                        if matches!(db.types[ty].kind, TypeKind::Record { .. }) =>
                    {
                        let (a, b) = two(self)?;
                        let mut pairs = Vec::new();
                        flatten(&a, b, &mut pairs)?;
                        let mut all: Option<Val> = None;
                        for (x, y) in pairs {
                            let e = match (x, y) {
                                (Val::Int(x), Val::Int(y)) => {
                                    Val::Part(part_of(self.int_cmp(Cmp::Eq, x, y), 1))
                                }
                                (x, y) => self.vec_eq(x, y, false, false)?,
                            };
                            all = Some(match all {
                                None => e,
                                Some(p) => self.gate(Gate::And, p, e)?,
                            });
                        }
                        let all = match all {
                            Some(v) => v,
                            None => Val::Part(self.bool_lit(true)),
                        };
                        if fam == Family::Neq {
                            self.not(all)
                        } else {
                            Ok(all)
                        }
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
                    let library = ["ieee.", "std."]
                        .iter()
                        .any(|p| sp.mangled.as_str().starts_with(p));
                    if !library {
                        // A subprogram of the design: its body, inlined.
                        return self
                            .inline(callee, args)?
                            .ok_or_else(|| "a function without a result".into());
                    }
                    let name = decl.name.key().as_str().to_ascii_lowercase();
                    if sp.mangled.as_str().starts_with("ieee.") && args.len() <= 2 {
                        match name.as_str() {
                            // A two-state value is never a metavalue.
                            "is_x" => return Ok(Val::Part(self.bool_lit(false))),
                            // Identities in two-state values (their index ranges are their own).
                            "to_stdulogic" | "to_bit" | "to_bitvector" | "to_stdlogicvector"
                            | "to_stdulogicvector" | "to_x01" | "to_x01z" | "to_ux01" | "to_01" => {
                                return one(self).map(no_dims);
                            }
                            _ => {}
                        }
                    }
                    // The logical operators of `numeric_bit` (and `numeric_std`) on its vectors,
                    // which have no native key: the gates of their elements.
                    let numeric = ["ieee.numeric_bit.", "ieee.numeric_std."]
                        .iter()
                        .any(|p| sp.mangled.as_str().starts_with(p));
                    let g = match decl.name.key().as_str() {
                        "\"and\"" => Some(Gate::And),
                        "\"or\"" => Some(Gate::Or),
                        "\"xor\"" => Some(Gate::Xor),
                        "\"nand\"" => Some(Gate::Nand),
                        "\"nor\"" => Some(Gate::Nor),
                        "\"xnor\"" => Some(Gate::Xnor),
                        _ => None,
                    };
                    return match (numeric, g, args.len()) {
                        (true, Some(g), 2) => {
                            let (a, b) = two(self)?;
                            self.gate(g, a, b).map(down_to_0)
                        }
                        (true, None, 1) if decl.name.key().as_str() == "\"not\"" => {
                            let v = one(self)?;
                            self.not(v).map(down_to_0)
                        }
                        // `numeric_std`'s `+` and `-` of a vector and a `std_ulogic`: the scalar
                        // as element 0 of a vector of zeros as long as the other (its body).
                        (true, None, 2)
                            if matches!(decl.name.key().as_str(), "\"+\"" | "\"-\"") =>
                        {
                            let sub = decl.name.key().as_str() == "\"-\"";
                            let (a, b) = (one(self)?, {
                                let [_, b] = args else { unreachable!() };
                                self.arg(b)?
                            });
                            let (a, b) = (self.part(a)?, self.part(b)?);
                            let n = a.len.max(b.len);
                            if a.len.min(b.len) != 1 || n < 1 || a.lit.is_some() || b.lit.is_some()
                            {
                                return not_yet();
                            }
                            let (a, b) = (self.ext(a, n, false), self.ext(b, n, false));
                            let (ra, rb) = (self.whole(a), self.whole(b));
                            let t = self.tmp(n);
                            self.ops.push(Op::Add {
                                dst: t.r,
                                a: ra,
                                b: rb,
                                len: n,
                                sub,
                            });
                            Ok(down_to_0(Val::Part(t)))
                        }
                        _ => not_yet(),
                    };
                };
                match op {
                    // `std_logic_1164`'s vectors are `1 to n`, `numeric_std`'s `n - 1 downto 0`.
                    I::Gate(g) => {
                        let (a, b) = two(self)?;
                        self.gate(gate(g), a, b)
                    }
                    I::VecGate(g) => {
                        let (a, b) = two(self)?;
                        self.gate(gate(g), a, b).map(one_to_n)
                    }
                    I::NumGate(g) => {
                        let (a, b) = two(self)?;
                        self.gate(gate(g), a, b).map(down_to_0)
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
                    I::Not => {
                        let v = one(self)?;
                        self.not(v)
                    }
                    I::VecNot => {
                        let v = one(self)?;
                        self.not(v).map(one_to_n)
                    }
                    I::NumNot => {
                        let v = one(self)?;
                        self.not(v).map(down_to_0)
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
                    I::ToSulv | I::Conv(..) | I::To01 | I::ToBitVector | I::Truthy => {
                        one(self).map(no_dims)
                    }
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
                        Ok(down_to_0(Val::Part(t)))
                    }
                    I::Mul { signed, form } => {
                        // The result has the lengths of both: twice the vector's with an integer.
                        let (a, b, m) = if form == Form::Vv {
                            let (a, b) = two(self)?;
                            let (a, b) = (self.part(a)?, self.part(b)?);
                            if a.lit.is_some() || b.lit.is_some() {
                                return Err(META_CMP.into());
                            }
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
                        if n <= 0 || n > VEC_MAX as i64 {
                            return Err("a null or very large vector size".into());
                        }
                        let a = self.arg(a)?;
                        let a = self.int(a)?;
                        if op == I::ToUnsigned {
                            self.natural(a);
                        }
                        Ok(down_to_0(Val::Part(self.int_vec(a, n as u32))))
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
                        if to <= 0 || to > VEC_MAX as i64 {
                            return Err("a null or very large vector size".into());
                        }
                        let to = to as u32;
                        let a = self.arg(a)?;
                        if let Val::Null(_) = a {
                            // `numeric_std`: a null argument gives `to` zeros.
                            let t = self.tmp(to);
                            self.ops.push(Op::Lit {
                                dst: t.r,
                                val: vec![0; words(to) as usize].into(),
                            });
                            return Ok(down_to_0(Val::Part(t)));
                        }
                        let a = self.part(a)?;
                        if !signed || to >= a.len {
                            return Ok(down_to_0(Val::Part(self.ext(a, to, signed))));
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
                        Ok(down_to_0(Val::Part(t)))
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

/// The outermost index range of an array value (of its first leaf for an array of records).
fn outer(v: &Val) -> Option<Bounds> {
    match v {
        Val::Part(p) => p.dims.first(),
        Val::Rec(fs) => outer(fs.first()?),
        Val::Int(_) => None,
        Val::Null(ds) => ds.first(),
    }
}

/// Whether array value `v` has no elements at all (a record of such fields included).
fn all_null(v: &Val) -> bool {
    match v {
        Val::Null(_) => true,
        Val::Rec(fs) => fs.iter().all(all_null),
        _ => false,
    }
}

/// The null slice `sb` of array value `v`: each leaf null, of index ranges `sb` and those of its
/// elements.
fn null_of(v: Val, sb: Bounds) -> R<Val> {
    match v {
        Val::Part(Part { at: Some(_), .. }) => {
            Err("a null slice of an element chosen at run time".into())
        }
        Val::Part(Part { dims, .. }) | Val::Null(dims) if dims.n > 0 => {
            Ok(Val::Null(dims.with_first(sb)))
        }
        Val::Rec(fs) => (fs.into_iter())
            .map(|f| null_of(f, sb))
            .collect::<R<Vec<_>>>()
            .map(Val::Rec),
        _ => Err("a slice of a value with no index range".into()),
    }
}

/// `f` on every leaf of array value `v`, all of outermost index range `b`.
fn at_each(v: Val, b: Bounds, f: &mut dyn FnMut(Part) -> Part) -> R<Val> {
    match v {
        Val::Part(p) if p.dims.first() == Some(b) => Ok(Val::Part(f(p))),
        // A field of no elements of an array of records: the index ranges of what `f` makes of
        // it (its index check is the other fields'; a value of null leaves alone is the
        // caller's).
        Val::Null(ds) if ds.first() == Some(b) => {
            let p = f(Part {
                r: Ref::Loc(u32::MAX),
                lo: 0,
                len: 0,
                full: 0,
                dims: ds,
                fresh: false,
                leaf: None,
                at: None,
                obj: None,
                lit: None,
                int: None,
            });
            Ok(Val::Null(p.dims))
        }
        Val::Rec(fs) => (fs.into_iter())
            .map(|x| at_each(x, b, f))
            .collect::<R<Vec<_>>>()
            .map(Val::Rec),
        _ => Err("an index or slice of a value with no index range".into()),
    }
}

/// Logic vector `v` with index range `r` when it has one.
fn with_range(v: Val, r: Option<Bounds>) -> Val {
    match (v, r) {
        (Val::Part(p), Some(b)) if u64::from(p.len) == b.len() => Val::Part(Part {
            dims: Dims::wrap(b, Dims::NONE).expect("one deep"),
            ..p
        }),
        (v, _) => v,
    }
}

/// A `std_logic_1164` result of `n` elements, whose index range the package fixes as `1 to n`.
fn one_to_n(v: Val) -> Val {
    let r = match &v {
        Val::Part(p) => Some(Bounds {
            left: 1,
            dir: Direction::To,
            right: i64::from(p.len),
        }),
        _ => None,
    };
    with_range(v, r)
}

/// `v` without index ranges: a function result whose range is not its argument's.
/// A `numeric_std` result of `n` elements, whose index range the package fixes as
/// `n - 1 downto 0` (`resize`, `to_unsigned`, `to_signed`).
fn down_to_0(v: Val) -> Val {
    match v {
        Val::Part(p) => Val::Part(Part {
            dims: Dims::wrap(
                Bounds {
                    left: i64::from(p.len) - 1,
                    dir: Direction::Downto,
                    right: 0,
                },
                Dims::NONE,
            )
            .expect("one deep"),
            ..p
        }),
        v => v,
    }
}

/// `v` as the value of object `d` (index check messages name it).
fn named(v: Val, d: DeclId) -> Val {
    match v {
        Val::Part(p) => Val::Part(Part { obj: Some(d), ..p }),
        Val::Rec(fs) => Val::Rec(fs.into_iter().map(|f| named(f, d)).collect()),
        v => v,
    }
}

fn no_dims(v: Val) -> Val {
    match v {
        Val::Part(p) => Val::Part(Part {
            dims: Dims::NONE,
            ..p
        }),
        v => v,
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
        int: None,
    }
}

impl Lx<'_> {
    fn int_arith(&mut self, x: ExprId, op: Arith, a: IntV, b: IntV) -> R<IntV> {
        let (lo, hi, ty) = self.result_range(x)?;
        let stmt = std::mem::replace(&mut self.at, self.db.exprs[x].span);
        let site = self.site(Check::Arith { op, ty });
        self.at = stmt;
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
        let mut rb = Ref::Edge;
        let right = self.capture(|lx| {
            let b = lx.arg(b)?;
            let b = lx.part(b)?;
            rb = lx.whole(b);
            Ok(())
        })?;
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
        for v in [&a, &b] {
            if let Val::Part(Part { lit: Some(_), .. }) = v {
                return Err(META_CMP.into());
            }
        }
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
        let vec = if form == Form::Vi { &a } else { &b };
        if let Val::Part(Part { lit: Some(_), .. }) = vec {
            return Err(META_CMP.into());
        }
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

/// Subprogram calls: the callee's body inlined into the caller, its formals bound to the
/// actuals' values and places (`out` and `inout` ones by reference, as the normal mode passes
/// them), its locals fresh per call.
impl Lx<'_> {
    /// What formal or local `d` of a call being inlined is, the innermost call first.
    fn bound(&self, d: DeclId) -> Option<Bind> {
        self.frames
            .iter()
            .rev()
            .find_map(|f| f.binds.get(&d).cloned())
    }

    fn proc_call(&mut self, call: &yavs_sem::hir::Call) -> R<()> {
        let db = self.db;
        if call.method_of.is_some() {
            return Err("a method of a protected object cannot run cycle-based".into());
        }
        let mut callee = call.callee;
        if let yavs_sem::decl::DeclKind::Alias(yavs_sem::decl::Alias::NonObject { target }) =
            &db.decls[callee].kind
        {
            callee = *target;
        }
        let library = (db.decls[callee].kind.subprogram()).is_none_or(|sp| {
            ["ieee.", "std."]
                .iter()
                .any(|p| sp.mangled.as_str().starts_with(p))
        });
        if library {
            return Err(format!(
                "a call of `{}`, which is not supported in cycle mode yet",
                db.decls[callee].name
            ));
        }
        self.inline(callee, &call.args).map(|_| ())
    }

    /// The body of `callee` for the call with `args`, inlined: a function's result.
    fn inline(&mut self, callee: DeclId, args: &[yavs_sem::hir::Arg]) -> R<Option<Val>> {
        use yavs_sem::decl::SubprogramKind;
        let db = self.db;
        let decl = &db.decls[callee];
        let not_yet = || {
            format!(
                "a call of `{}`, which is not supported in cycle mode yet",
                decl.name
            )
        };
        let Some(sp) = decl.kind.subprogram() else {
            return Err(not_yet());
        };
        let yavs_sem::decl::Body::Hir { scope, stmts } = &sp.body else {
            return Err(not_yet());
        };
        if sp.generics.is_some() {
            return Err(not_yet());
        }
        if self.frames.len() >= DEPTH_MAX {
            return Err(format!(
                "calls of `{}` nested more than {DEPTH_MAX} deep (a recursion elaboration does \
                 not bound)",
                decl.name
            ));
        }
        let mut all = Vec::new();
        deep(db, stmts, &mut all);
        // Every inlined statement counts against the budget of unrolled ones.
        let size = all.len() as u32 + 1;
        if self.unrolled.saturating_add(size) > UNROLL_MAX {
            return Err(format!(
                "calls of `{}` that inline more than cycle mode takes ({UNROLL_MAX} statements)",
                decl.name
            ));
        }
        self.unrolled += size;
        let procedure = sp.kind == SubprogramKind::Procedure;
        let pure = sp.kind == SubprogramKind::Function { pure: true };
        let mut binds = self.formals(callee, sp, args, !pure)?;
        // A body apart from its declaration declares the formals again: the same, by position.
        let again: Vec<DeclId> = (db.scopes[*scope].order.iter().copied())
            .filter(|&d| {
                matches!(&db.decls[d].kind, yavs_sem::decl::DeclKind::Object(o)
                    if o.role == yavs_sem::decl::ObjRole::Param)
            })
            .collect();
        if !again.is_empty() {
            if again.len() != sp.params.len() {
                return Err(not_yet());
            }
            for (d, p) in again.iter().zip(sp.params.iter()) {
                let b = p
                    .decl
                    .and_then(|x| binds.get(&x).cloned())
                    .ok_or_else(not_yet)?;
                binds.insert(*d, b);
            }
        }
        let returns: Vec<StmtId> = (all.iter().copied())
            .filter(|&x| matches!(db.stmts[x].kind, StmtKind::Return(_)))
            .collect();
        // One `return` that ends the body: its value is the result, no copy.
        let direct = !procedure && returns.len() == 1 && stmts.last() == returns.first();
        // A `return` inside another statement: what follows it runs behind a flag.
        let flag = returns
            .iter()
            .any(|r| !stmts.contains(r))
            .then(|| self.loc(1));
        if let Some(f) = flag {
            self.ops.push(Op::Lit {
                dst: f,
                val: Box::new([0]),
            });
        }
        let depth = self.frames.len();
        let base = self.locs;
        self.frames.push(Frame {
            callee,
            binds,
            ret_st: sp.ret,
            ret: None,
            direct,
            from: self.at,
            flag,
        });
        let saved = std::mem::take(&mut self.pend);
        let r = match self.locals(*scope) {
            Ok(()) => self.stmts(stmts),
            e => e,
        };
        let frame = self.frames.pop().expect("pushed above");
        // The call's own words are not read again.
        self.known.retain(|w, _| *w < base);
        let jumps = std::mem::replace(&mut self.pend, saved);
        r?;
        if jumps.iter().any(|j| j.tgt != Tgt::Ret(depth)) {
            return Err("a jump cycle mode cannot follow".into());
        }
        if !procedure && !jumps.iter().any(|j| j.always) {
            return Err(format!(
                "`{}` may reach its end without `return`, which cycle mode does not model",
                decl.name
            ));
        }
        Ok(frame.ret)
    }

    /// The formals of a call (`copy`: scalar `in` values are copied, since the callee may change
    /// their actual).
    fn formals(
        &mut self,
        callee: DeclId,
        sp: &yavs_sem::decl::Subprogram,
        args: &[yavs_sem::hir::Arg],
        copy: bool,
    ) -> R<HashMap<DeclId, Bind>> {
        use yavs_sem::decl::ObjClass;
        use yavs_sem::hir::Mode;
        let db = self.db;
        let name = &db.decls[callee].name;
        let mut binds = HashMap::new();
        for (i, p) in sp.params.iter().enumerate() {
            let d = p.decl.ok_or("a parameter without a declaration")?;
            let what = format!("parameter {}", p.name.text.as_str().to_ascii_uppercase());
            let x = match args
                .iter()
                .find(|a| usize::from(a.formal) == i)
                .map(|a| &a.actual)
            {
                Some(Actual::Expr(x)) => *x,
                None | Some(Actual::Default) => p
                    .default
                    .ok_or_else(|| format!("a call of `{name}` without a value for its {what}"))?,
                Some(_) => {
                    return Err(
                        "a call with an open, converted or individual actual is not supported in \
                         cycle mode yet"
                            .into(),
                    );
                }
            };
            let b = if p.class == ObjClass::Signal {
                let v = self.name(x, p.mode != Mode::In)?;
                if !self.signal_rooted(&v) {
                    return Err("a signal parameter whose actual is not a signal".into());
                }
                Bind::Val(self.formal_val(v, p.subtype, Some(d), &what)?)
            } else if p.mode == Mode::In {
                match self.fold(x) {
                    Some(c @ (ConstValue::Int(_) | ConstValue::Enum(_))) => {
                        if let Some(v) = c.as_int()
                            && let Ok(b) = self.subtype_bounds(p.subtype)
                            && b.pos(v).is_none()
                        {
                            return Err(format!("a value outside the subtype of {what}"));
                        }
                        Bind::Const(c)
                    }
                    _ => {
                        let v = self.expr(x)?;
                        let v = if copy { self.copy_scalar(v) } else { v };
                        Bind::Val(unfresh(self.formal_val(v, p.subtype, Some(d), &what)?))
                    }
                }
            } else {
                // `out` and `inout` variables by reference, as the normal mode passes them: the
                // formal's subtype checks what the callee writes.
                let place = self.name(x, true)?;
                if self.signal_rooted(&place) {
                    return Err("a variable parameter whose actual is a signal".into());
                }
                // ponytail: an integer formal claims its own range for what it reads, which an
                // `out` actual of a wider subtype may not hold (an erroneous call); a separate
                // check range in `IntV` would make reads exact.
                Bind::Val(unfresh(match place {
                    Val::Int(a) => {
                        let b = self.subtype_bounds(p.subtype)?;
                        let (lo, hi) = b.low_high();
                        Val::Int(IntV {
                            lo,
                            hi,
                            down: b.dir == Direction::Downto,
                            place: Some((p.subtype, d)),
                            ..a
                        })
                    }
                    v => self.formal_val(v, p.subtype, Some(d), &what)?,
                }))
            };
            binds.insert(d, b);
        }
        Ok(binds)
    }

    /// A scalar value copied into a temporary of its own (an `in` parameter of a procedure).
    fn copy_scalar(&mut self, v: Val) -> Val {
        match v {
            Val::Int(i) if !i.fresh => {
                let t = IntV {
                    r: self.loc(1),
                    place: None,
                    leaf: None,
                    ..i
                };
                self.ops.push(Op::Copy {
                    dst: t.r,
                    src: i.r,
                    len: 64,
                });
                Val::Int(t)
            }
            Val::Part(p) if !p.fresh && p.len == 1 && p.dims.n == 0 => {
                let s = self.whole(p);
                let t = self.tmp(1);
                self.ops.push(Op::Copy {
                    dst: t.r,
                    src: s,
                    len: 1,
                });
                Val::Part(t)
            }
            v => v,
        }
    }

    /// `v` as a value of subtype `st` (a formal, a result): an integer checked against its range
    /// (message `what`), an array given the subtype's index ranges when it has them.
    fn formal_val(&mut self, v: Val, st: SubtypeId, obj: Option<DeclId>, what: &str) -> R<Val> {
        match v {
            Val::Int(i) => {
                let b = self.subtype_bounds(st)?;
                let (lo, hi) = b.low_high();
                if i.lo < lo || i.hi > hi {
                    let site = self.site(Check::Range {
                        ty: subtype_name(self.db, st),
                        lo,
                        hi,
                        down: b.dir == Direction::Downto,
                        what: what.into(),
                    });
                    self.ops.push(Op::Range {
                        a: i.r,
                        lo,
                        hi,
                        site,
                    });
                }
                Ok(Val::Int(IntV {
                    lo: i.lo.max(lo),
                    hi: i.hi.min(hi),
                    place: None,
                    ..i
                }))
            }
            Val::Part(p) => {
                let mut p = self.rebound(p, st)?;
                if obj.is_some() {
                    p.obj = obj;
                }
                Ok(Val::Part(p))
            }
            v @ Val::Rec(_) => Ok(v),
            Val::Null(ds) => self.rebound_null(ds, st),
        }
    }

    /// A null array as a value of subtype `st`: of its index ranges, which must be null too.
    fn rebound_null(&mut self, ds: Dims, st: SubtypeId) -> R<Val> {
        let db = self.db;
        let t = db.base(st);
        let c = &db.subtypes[st].constraint;
        if !matches!(db.types[t].kind, TypeKind::Array { .. }) || self.open_array(t, c) {
            return Ok(Val::Null(ds));
        }
        let to = self.arr_dims(t, c)?;
        if to.b[0].len() * u64::from(to.elem_len()) != 0 {
            return Err("a value of another length than its subtype".into());
        }
        Ok(Val::Null(to))
    }

    /// `p` as a value of subtype `st`: a constrained array subtype gives it its index ranges
    /// (its length checked during elaboration), an open one leaves the value's.
    fn rebound(&mut self, mut p: Part, st: SubtypeId) -> R<Part> {
        let db = self.db;
        let t = db.base(st);
        let c = &db.subtypes[st].constraint;
        if !matches!(db.types[t].kind, TypeKind::Array { .. }) || self.open_array(t, c) {
            return Ok(p);
        }
        let ds = self.arr_dims(t, c)?;
        if u64::from(p.len) != ds.b[0].len() * u64::from(ds.elem_len()) {
            return Err("a value of another length than its subtype".into());
        }
        p.dims = ds;
        Ok(p)
    }

    /// Neither constraint `c` nor array type `t` itself gives the index range.
    fn open_array(&self, t: TypeId, c: &Constraint) -> bool {
        let dims = |c: &Constraint| matches!(c, Constraint::Array { dims: Some(_), .. });
        !dims(c) && !dims(&self.db.subtypes[self.db.types[t].base_subtype].constraint)
    }

    /// The index ranges of array type `t` under constraint `c` (or the type's own).
    fn arr_dims(&mut self, t: TypeId, c: &Constraint) -> R<Dims> {
        let db = self.db;
        let TypeKind::Array { indexes, elem } = &db.types[t].kind else {
            return Err("an array value of a type that is not an array".into());
        };
        if indexes.len() != 1 {
            return Err("a multi-dimensional array is not supported in cycle mode yet".into());
        }
        let own = |c: &'_ Constraint| match c {
            Constraint::Array { dims: Some(d), .. } => Some(d.clone()),
            _ => None,
        };
        let dims = own(c)
            .or_else(|| own(&db.subtypes[db.types[t].base_subtype].constraint))
            .ok_or("an array whose index range is not known here")?;
        let [r] = &dims[..] else {
            return Err("a multi-dimensional array is not supported in cycle mode yet".into());
        };
        let (outer, _) = self.static_range(r)?;
        let ec = match c {
            Constraint::Array { elem: ec, .. } if !matches!(**ec, Constraint::None) => ec,
            _ => &db.subtypes[*elem].constraint,
        };
        let et = db.base(*elem);
        let inner = match &db.types[et].kind {
            TypeKind::Enum { .. } if yavs_elab::elem_of(db, et) == yavs_kernel::Elem::Logic => {
                Dims::NONE
            }
            TypeKind::Array { .. } => self.arr_dims(et, ec)?,
            _ => {
                return Err(
                    "an array whose elements are not logic values or arrays of them is not \
                     supported in cycle mode yet"
                        .into(),
                );
            }
        };
        Dims::wrap(outer, inner).ok_or_else(|| "an array nested more than four deep".into())
    }

    /// A fresh place of subtype `st` for object `d` (a local of an inlined call).
    fn layout(&mut self, st: SubtypeId, d: DeclId) -> R<Val> {
        let db = self.db;
        let t = db.base(st);
        match &db.types[t].kind {
            TypeKind::Enum { lits } if yavs_elab::elem_of(db, t) == yavs_kernel::Elem::Logic => {
                if self.subtype_bounds(st)?.len() != lits.len() as u64 {
                    return Err(
                        "a logic subtype with a range (`X01`, `UX01`, ...) is not supported in \
                         cycle mode yet"
                            .into(),
                    );
                }
                let mut p = self.tmp(1);
                (p.fresh, p.obj) = (false, Some(d));
                Ok(Val::Part(p))
            }
            TypeKind::Integer | TypeKind::Enum { .. } => {
                let b = self.subtype_bounds(st)?;
                let (lo, hi) = b.low_high();
                Ok(Val::Int(IntV {
                    r: self.loc(1),
                    lo,
                    hi,
                    down: b.dir == Direction::Downto,
                    place: Some((st, d)),
                    fresh: false,
                    leaf: None,
                }))
            }
            TypeKind::Array { .. } => {
                let ds = self.arr_dims(t, &db.subtypes[st].constraint)?;
                let n = ds.b[0].len() * u64::from(ds.elem_len());
                if n == 0 || n > VEC_MAX {
                    return Err(format!(
                        "a vector of {n} elements (cycle mode takes 1 to 2**26 for now)"
                    ));
                }
                let mut p = self.tmp(n as u32);
                (p.dims, p.fresh, p.obj) = (ds, false, Some(d));
                Ok(Val::Part(p))
            }
            TypeKind::Record { elems } => {
                if matches!(&db.subtypes[st].constraint, Constraint::Record { elems } if !elems.is_empty())
                {
                    return Err(
                        "a record subtype with element constraints is not supported in cycle \
                         mode yet"
                            .into(),
                    );
                }
                (elems.iter())
                    .map(|el| self.layout(el.subtype, d))
                    .collect::<R<Vec<_>>>()
                    .map(Val::Rec)
            }
            _ => {
                Err("a value of a real or physical type is not supported in cycle mode yet".into())
            }
        }
    }

    /// Every leaf of `v` at its start value: logic 0 (`'U'`, counted), an integer its `'left`.
    fn start(&mut self, v: &Val) {
        match v {
            Val::Part(p) => {
                self.uninit += u64::from(p.len);
                self.ops.push(Op::Lit {
                    dst: p.r,
                    val: vec![0; words(p.full) as usize].into(),
                });
            }
            Val::Int(i) => self.ops.push(Op::IntLit {
                dst: i.r,
                v: if i.down { i.hi } else { i.lo },
            }),
            Val::Rec(fs) => fs.iter().for_each(|f| self.start(f)),
            Val::Null(_) => {}
        }
    }

    /// The declarations of an inlined body: variables and constants fresh for this call.
    fn locals(&mut self, scope: yavs_sem::ScopeId) -> R<()> {
        use yavs_sem::decl::{DeclKind, ObjClass};
        let db = self.db;
        for &d in &db.scopes[scope].order {
            if self.frames.last().is_some_and(|f| f.binds.contains_key(&d)) {
                continue;
            }
            let decl = &db.decls[d];
            let b = match &decl.kind {
                DeclKind::Object(o) if o.class == ObjClass::Variable => {
                    let v = self.layout(o.subtype, d)?;
                    match o.default {
                        Some(init) => {
                            let c = self.fold(init);
                            let s = self.expr_h(init, Some(&v))?;
                            self.assign(&v, s)?;
                            self.learn(&v, c);
                        }
                        None => self.start(&v),
                    }
                    Bind::Val(v)
                }
                DeclKind::Object(o) if o.class == ObjClass::Constant => {
                    let init = o
                        .default
                        .ok_or_else(|| format!("the constant `{}` without a value", decl.name))?;
                    match self.fold(init) {
                        Some(c @ (ConstValue::Int(_) | ConstValue::Enum(_))) => Bind::Const(c),
                        _ => {
                            let open =
                                matches!(db.types[db.base(o.subtype)].kind, TypeKind::Array { .. })
                                    && self.open_array(
                                        db.base(o.subtype),
                                        &db.subtypes[o.subtype].constraint,
                                    );
                            let s = self.expr(init)?;
                            let v = if open {
                                self.fresh_like(&s)
                            } else {
                                self.layout(o.subtype, d)?
                            };
                            self.assign(&v, s)?;
                            Bind::Val(v)
                        }
                    }
                }
                DeclKind::Type(_)
                | DeclKind::Subtype(_)
                | DeclKind::Subprogram(_)
                | DeclKind::EnumLit { .. }
                | DeclKind::Attribute { .. } => continue,
                _ => {
                    return Err(format!(
                        "the declaration of `{}` in a subprogram, which cycle mode does not take \
                         yet",
                        decl.name
                    ));
                }
            };
            self.frames
                .last_mut()
                .expect("in a call")
                .binds
                .insert(d, b);
        }
        Ok(())
    }

    /// A fresh place of the shape of `v`.
    fn fresh_like(&mut self, v: &Val) -> Val {
        match v {
            Val::Part(p) => {
                let mut t = self.tmp(p.len);
                (t.dims, t.fresh) = (p.dims, false);
                Val::Part(t)
            }
            Val::Int(i) => Val::Int(IntV {
                r: self.loc(1),
                place: None,
                fresh: false,
                leaf: None,
                ..*i
            }),
            Val::Rec(fs) => Val::Rec(fs.iter().map(|f| self.fresh_like(f)).collect()),
            Val::Null(ds) => Val::Null(*ds),
        }
    }

    /// `return [v];` of the innermost call.
    fn ret(&mut self, v: Option<ExprId>) -> R<()> {
        let Some(fi) = self.frames.len().checked_sub(1) else {
            return Err("a `return` statement in a process".into());
        };
        match (v, self.frames[fi].ret_st) {
            (Some(e), Some(st)) => {
                let hint = self.frames[fi].ret.clone();
                let val = self.expr_h(e, hint.as_ref())?;
                let name = self.db.decls[self.frames[fi].callee].name.to_string();
                let val = self.formal_val(val, st, None, &format!("the result of {name}"))?;
                match (self.frames[fi].direct, self.frames[fi].ret.clone()) {
                    (true, _) => self.frames[fi].ret = Some(val),
                    (false, None) => {
                        let slot = self.fresh_like(&val);
                        self.assign(&slot, val)?;
                        self.frames[fi].ret = Some(slot);
                    }
                    (false, Some(slot)) => {
                        // Results of other index ranges: the result has none known.
                        if let (Val::Part(a), Val::Part(b)) = (&slot, &val)
                            && dims_differ(&a.dims, &b.dims)
                        {
                            self.frames[fi].ret = Some(no_dims(slot.clone()));
                        }
                        self.assign(&slot, val)?;
                    }
                }
            }
            (None, None) => {}
            _ => return Err("a `return` that does not match its subprogram".into()),
        }
        let flag = self.frames[fi].flag;
        if let Some(f) = flag {
            self.ops.push(Op::Lit {
                dst: f,
                val: Box::new([1]),
            });
        }
        self.pend.push(Jump {
            tgt: Tgt::Ret(fi),
            flag,
            always: true,
        });
        Ok(())
    }
}

/// Two lists of index ranges that are not the same.
fn dims_differ(a: &Dims, b: &Dims) -> bool {
    a.n != b.n
        || (a.b[..a.n as usize].iter().zip(&b.b[..b.n as usize]))
            .any(|(x, y)| (x.left, x.dir, x.right) != (y.left, y.dir, y.right))
}

/// `v` as a value that is named more than once: no operation may be retargeted to make it.
fn unfresh(v: Val) -> Val {
    match v {
        Val::Part(p) => Val::Part(Part { fresh: false, ..p }),
        Val::Int(i) => Val::Int(IntV { fresh: false, ..i }),
        Val::Rec(fs) => Val::Rec(fs.into_iter().map(unfresh).collect()),
        v @ Val::Null(_) => v,
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
        DeclKind::Object(o) if o.role == yavs_sem::decl::ObjRole::Param => {
            format!("parameter {name}")
        }
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

/// The value an unknown name with `key` takes in trial `t` of `Lx::probe`: 0 in trial 0, else one
/// of 1 to 4096 mixed from the key and the trial.
fn trial_value(key: u64, t: u64) -> i64 {
    if t == 0 {
        return 0;
    }
    let mut z = key.wrapping_add(t.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    ((z ^ (z >> 31)) % 4096) as i64 + 1
}
