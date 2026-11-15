//! The bodies of member processes, from the elaborated HIR to region operations on two-state
//! words: a logic value of `len` elements is `ceil(len / 64)` words (element 0 in bit 0, the bits
//! above `len` zero), an integer one word. Metavalues read as 0: a literal's are counted per site,
//! a variable's missing initial value per element. Anything else is a reason to leave the process
//! in the event kernel, never a guess.

use std::collections::{BTreeSet, HashMap};

use yavs_elab::{Bounds, Elaborated, LeafId, ProcInst, Shape};
use yavs_sem::hir::{Actual, ExprKind, Rhs, StmtKind, Waveform};
use yavs_sem::implicit::Family;
use yavs_sem::stat::{ConstValue, Elems};
use yavs_sem::types::{Direction, RangeC, TypeKind};
use yavs_sem::{Db, DeclId, ExprId, StmtId, SubtypeId};

use crate::codegen::{mask, words};

/// Words of the region's state: the current value of a kernel signal (all the leaves the kernel
/// keeps in it, each at its kernel offset), or a slot of the process (a leaf of a variable, a
/// temporary), numbered per body.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum Ref {
    Sig(u32),
    Loc(u32),
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
    /// `a = b` (or `/=`) into a boolean.
    VecEq {
        dst: Ref,
        a: Ref,
        b: Ref,
        len: u32,
        ne: bool,
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
    /// Element `idx` of the vector of `n` elements from element `lo` of `src`, index range
    /// from `left` (`down`: descending), checked.
    IndexRead {
        dst: Ref,
        src: Ref,
        lo: u32,
        n: u32,
        idx: Ref,
        left: i64,
        down: bool,
        site: u32,
    },
    /// The boolean `src` into element `idx` of such a vector of `dst`.
    IndexWrite {
        dst: Ref,
        src: Ref,
        lo: u32,
        n: u32,
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
            | Op::VecEq { dst, .. }
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
    pub ops: Vec<Op>,
    /// Signal leaves read and written.
    pub reads: BTreeSet<LeafId>,
    pub writes: BTreeSet<LeafId>,
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
    /// The index range of an array part (`None`: a scalar, or a value with no name).
    bounds: Option<Bounds>,
    /// A temporary made for this value alone: the operation that made it may write elsewhere.
    fresh: bool,
    /// The signal leaf the part is of (reads and writes of the body are counted by leaf).
    leaf: Option<LeafId>,
    /// One element of the vector from element `lo`, chosen at run time: the index and the
    /// vector's index range.
    at: Option<(Ref, Bounds)>,
    /// The object a name starts from (messages of index checks).
    obj: Option<DeclId>,
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

/// Lower process `pi` if its shape and every construct of its body are in the subset.
pub(crate) fn lower(db: &Db, e: &Elaborated, pi: usize) -> R<Body> {
    let p = &e.procs[pi];
    if p.actual.is_some() {
        return Err("a port expression actual is not supported in cycle mode yet".into());
    }
    if p.postponed {
        return Err("a postponed process cannot run cycle-based".into());
    }
    let StmtKind::Process {
        sensitivity,
        body,
        scope,
        ..
    } = &db.stmts[p.stmt].kind
    else {
        return Err("concurrent statements are not supported in cycle mode yet".into());
    };
    let mut lx = Lx {
        db,
        e,
        p,
        clocked: p.trigger.is_some(),
        vars: HashMap::new(),
        ops: Vec::new(),
        reads: BTreeSet::new(),
        writes: BTreeSet::new(),
        locs: 0,
        init: Vec::new(),
        var_words: Vec::new(),
        dropped: 0,
        metas: 0,
        uninit: 0,
        checks: Vec::new(),
    };
    // `if <edge> then ... end if;` with the kernel's edge trigger.
    if let Some(t) = p.trigger {
        if t.wait {
            return Err("a clocked `wait until` process is not supported in cycle mode yet".into());
        }
        if t.reset.is_some() {
            return Err("an asynchronous reset is not supported in cycle mode yet".into());
        }
        let [s] = &body[..] else {
            return Err("a clocked process with statements outside its edge test".into());
        };
        let StmtKind::If {
            branches,
            else_: None,
        } = &db.stmts[*s].kind
        else {
            return Err("a clocked process with an `else` or `elsif` branch".into());
        };
        let [(cond, inner)] = &branches[..] else {
            return Err("a clocked process with an `elsif` branch".into());
        };
        if db.edge_shape(*cond).is_none() {
            return Err("a clocked process whose condition is more than the edge test".into());
        }
        lx.stmts(inner)?;
        let clock = Clock {
            leaf: t.leaf,
            elem: t.elem,
            rising: t.rising,
            legacy: t.legacy,
        };
        return Ok(lx.body(Kind::Clocked, Some(clock)));
    }
    if matches!(sensitivity, yavs_sem::hir::Sensitivity::None) {
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
    if db.pure_comb_reads(*scope, &kept).is_none() {
        return Err(
            "a combinational process that keeps state in a variable or does more than assign \
             signals and variables"
                .into(),
        );
    }
    lx.stmts(body)?;
    // Cycle mode computes from every read; the normal mode only wakes on the list.
    if let yavs_sem::hir::Sensitivity::List(list) = sensitivity {
        let mut listed = BTreeSet::new();
        for l in list.iter() {
            let yavs_sem::hir::ObjRoot::Decl(d) = l.root else {
                return Err("a sensitivity list naming an external name".into());
            };
            // A part names fewer leaves than its object: not taken yet (the check must not
            // count leaves the list does not name).
            if !l.path.is_empty() {
                return Err(
                    "a sensitivity list naming part of a signal is not supported in cycle mode \
                     yet"
                    .into(),
                );
            }
            let Some(o) = e.signal(p.scope, d) else {
                return Err("a sensitivity list naming something that is not a signal".into());
            };
            let (a, z) = e.nets.obj(o).leaves;
            listed.extend((a.0..z.0).map(LeafId));
        }
        if let Some(l) = lx.reads.iter().find(|l| !listed.contains(l)) {
            return Err(format!(
                "its sensitivity list does not name {}, which it reads",
                e.leaf_name(*l)
            ));
        }
    }
    // A combinational variable is assigned before any read (`pure_comb_reads`): its start
    // value is never seen.
    lx.uninit = 0;
    Ok(lx.body(Kind::Comb, None))
}

struct Lx<'a> {
    db: &'a Db,
    e: &'a Elaborated,
    p: &'a ProcInst,
    clocked: bool,
    /// The leaves of each process variable met so far.
    vars: HashMap<DeclId, Val>,
    ops: Vec<Op>,
    reads: BTreeSet<LeafId>,
    writes: BTreeSet<LeafId>,
    locs: u32,
    init: Vec<(u32, u64)>,
    var_words: Vec<(u32, u32)>,
    dropped: u32,
    metas: u32,
    uninit: u64,
    checks: Vec<Check>,
}

impl Lx<'_> {
    fn body(self, kind: Kind, clock: Option<Clock>) -> Body {
        Body {
            kind,
            clock,
            ops: self.ops,
            reads: self.reads,
            writes: self.writes,
            locs: self.locs,
            init: self.init,
            vars: self.var_words,
            dropped: self.dropped,
            metas: self.metas,
            uninit: self.uninit,
            checks: self.checks,
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
            bounds: None,
            fresh: true,
            leaf: None,
            at: None,
            obj: None,
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
            StmtKind::VarAssign {
                target,
                rhs: Rhs::Simple(v),
            } => {
                let dst = self.name(target.expr, true)?;
                if self.signal_rooted(&dst) {
                    return Err("an assignment target that is not a variable".into());
                }
                let src = self.expr(*v)?;
                self.assign(&dst, src)
            }
            StmtKind::SignalAssign { target, delay, rhs } => {
                if matches!(delay, yavs_sem::hir::Delay::Inertial { reject: Some(_) }) {
                    return Err("a `reject` limit has no meaning in cycle mode".into());
                }
                let Rhs::Simple(Waveform::Elements(w)) = rhs else {
                    return Err(
                        "a conditional or selected signal assignment is not supported in cycle \
                         mode yet"
                            .into(),
                    );
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
                let src = self.expr(v)?;
                self.assign(&dst, src)
            }
            // A combinational `if` can leave a signal unassigned (a latch), which milestone 3
            // checks; in a clocked process that only keeps the register.
            StmtKind::If { branches, else_ } if self.clocked => {
                self.if_chain(branches, else_.as_deref())
            }
            StmtKind::If { .. } => {
                Err("an `if` statement in the body is not supported in cycle mode yet".into())
            }
            StmtKind::Case { .. } => {
                Err("a `case` statement is not supported in cycle mode yet".into())
            }
            StmtKind::Loop { .. } => Err("a loop is not supported in cycle mode yet".into()),
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
            _ => Err("a statement that is not supported in cycle mode yet".into()),
        }
    }

    /// `if c1 then .. elsif c2 then .. else .. end if` as nested `Op::If`.
    fn if_chain(
        &mut self,
        branches: &[(ExprId, Box<[StmtId]>)],
        else_: Option<&[StmtId]>,
    ) -> R<()> {
        let [(c, body), rest @ ..] = branches else {
            return else_.map_or(Ok(()), |e| self.stmts(e));
        };
        let c = self.expr(*c)?;
        let c = self.part(c)?;
        if c.len != 1 {
            return Err("a condition that is not a boolean".into());
        }
        let cond = self.whole(c);
        let outer = std::mem::take(&mut self.ops);
        self.stmts(body)?;
        let then = std::mem::take(&mut self.ops);
        self.if_chain(rest, else_)?;
        let else_ = std::mem::replace(&mut self.ops, outer);
        self.ops.push(Op::If { cond, then, else_ });
        Ok(())
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
                if let Some((idx, b)) = d.at {
                    let s = self.whole(s);
                    let site = self.index_site(d, b);
                    self.ops.push(Op::IndexWrite {
                        dst: d.r,
                        src: s,
                        lo: d.lo,
                        n: b.len() as u32,
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
            (Val::Int(d), Val::Int(s)) => {
                if let Some(l) = d.leaf {
                    self.writes.insert(l);
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
        let Some((idx, b)) = p.at else {
            return p;
        };
        let site = self.index_site(&p, b);
        let t = self.tmp(1);
        self.ops.push(Op::IndexRead {
            dst: t.r,
            src: p.r,
            lo: p.lo,
            n: b.len() as u32,
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

    /// A name: a signal, a variable of the process, or a field, element or slice of one.
    /// `target`: the name of an assignment target (its signals are not reads).
    fn name(&mut self, x: ExprId, target: bool) -> R<Val> {
        let db = self.db;
        match &db.exprs[x].kind {
            ExprKind::Obj(d) => self.object(*d, target),
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
                let b = p.bounds.ok_or("an index of a value with no index range")?;
                match self.static_int(*i) {
                    Ok(i) => {
                        let el = elem_of(b, i).ok_or("an index outside its array's range")?;
                        p.lo += el;
                    }
                    Err(_) => {
                        let i = self.expr(*i)?;
                        let i = self.int(i)?;
                        p.at = Some((i.r, b));
                    }
                }
                p.len = 1;
                p.bounds = None;
                Ok(Val::Part(p))
            }
            ExprKind::Slice { prefix, range } => {
                let RangeC::Explicit { left, dir, right } = range else {
                    return Err(
                        "a slice by a range attribute is not supported in cycle mode yet".into(),
                    );
                };
                let (l, r) = (self.static_int(*left)?, self.static_int(*right)?);
                let mut p = self.array_part(*prefix, target)?;
                let b = p.bounds.ok_or("a slice of a value with no index range")?;
                let sb = Bounds {
                    left: l,
                    dir: *dir,
                    right: r,
                };
                if sb.dir != b.dir {
                    return Err("a slice in the other direction than its array".into());
                }
                let n = sb.len() as u32;
                if n == 0 {
                    return Err("a null slice".into());
                }
                // The slice's rightmost element is its element 0.
                let lo = elem_of(b, r).ok_or("a slice outside its array's range")?;
                elem_of(b, l).ok_or("a slice outside its array's range")?;
                p.lo += lo;
                p.len = n;
                p.bounds = Some(sb);
                Ok(Val::Part(p))
            }
            _ => Err("a name that is not supported in cycle mode yet".into()),
        }
    }

    fn array_part(&mut self, prefix: ExprId, target: bool) -> R<Part> {
        match self.name(prefix, target)? {
            Val::Part(p) if p.at.is_none() => Ok(p),
            Val::Part(_) => Err("an index or slice of an element chosen at run time".into()),
            _ => Err("an index or slice of a record or an integer".into()),
        }
    }

    fn object(&mut self, d: DeclId, target: bool) -> R<Val> {
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
                if !so.parts.is_empty() {
                    return Err(format!(
                        "the port `{}`, collapsed into part of its actual",
                        decl.name
                    ));
                }
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
                let v = self.place_sig(v)?;
                if !target {
                    self.note_reads(&v);
                }
                Ok(v)
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
                if o.default.is_some() {
                    return Err(format!(
                        "the variable `{}` has an initial value, which is not supported in cycle \
                         mode yet",
                        decl.name
                    ));
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
                let v = self.place_vars(v);
                self.vars.insert(d, v.clone());
                Ok(v)
            }
            ObjClass::Constant => Err(format!(
                "the constant `{}`, which is not supported in cycle mode yet unless folded",
                decl.name
            )),
            _ => Err(format!(
                "`{}`, an object of a class cycle mode does not take",
                decl.name
            )),
        }
    }

    /// Give a variable's leaves their slots: a logic leaf starts at 0 (its `'U'`, counted), an
    /// integer at its `'left`.
    fn place_vars(&mut self, v: Val) -> Val {
        match v {
            Val::Part(mut p) => {
                p.r = self.loc(words(p.full));
                self.uninit += u64::from(p.full);
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
                {
                    self.init.push((w, left as u64));
                }
                Val::Int(i)
            }
            Val::Rec(fs) => Val::Rec(fs.into_iter().map(|f| self.place_vars(f)).collect()),
        }
    }

    /// A signal's leaves (numbered `Loc(leaf)` by `tree`) at their kernel signal and offset.
    fn place_sig(&self, v: Val) -> R<Val> {
        let nets = &self.e.nets;
        let leaf_of = |r: Ref| match r {
            Ref::Loc(l) => Ok(LeafId(l)),
            Ref::Sig(_) => Err("a signal leaf placed twice".to_string()),
        };
        match v {
            Val::Part(mut p) => {
                let l = leaf_of(p.r)?;
                let leaf = nets.leaf(l);
                if leaf.ty.len != p.len || leaf.ty.elem != yavs_kernel::Elem::Logic {
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
                p.lo = at.base;
                p.full = nets.signal_len(at.signal);
                p.leaf = Some(l);
                Ok(Val::Part(p))
            }
            Val::Int(mut i) => {
                let l = leaf_of(i.r)?;
                let leaf = nets.leaf(l);
                let at = nets.loc[l.index()];
                if leaf.ty.len != 1
                    || !matches!(leaf.ty.elem, yavs_kernel::Elem::Bytes(4 | 8))
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
                .map(|f| self.place_sig(f))
                .collect::<R<Vec<_>>>()
                .map(Val::Rec),
        }
    }

    fn note_reads(&mut self, v: &Val) {
        match v {
            Val::Part(Part { leaf: Some(l), .. }) | Val::Int(IntV { leaf: Some(l), .. }) => {
                self.reads.insert(*l);
            }
            Val::Part(_) | Val::Int(_) => {}
            Val::Rec(fs) => fs.iter().for_each(|f| self.note_reads(f)),
        }
    }

    /// The leaves of `shape` of object `d`: records of logic scalars, one-dimensional logic
    /// arrays and integers.
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
                    bounds: None,
                    fresh: false,
                    leaf: None,
                    at: None,
                    obj: Some(d),
                }))
            }
            Shape::Scalar {
                ty,
                sub,
                range: Some(b),
                ..
            } if matches!(db.types[*ty].kind, TypeKind::Integer) => {
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
            Shape::Scalar { .. } => Err(
                "a value of an enumeration or real type is not supported in cycle mode yet".into(),
            ),
            Shape::Array { dims, elem, .. } => {
                let (
                    [b],
                    Shape::Scalar {
                        elem: yavs_kernel::Elem::Logic,
                        ..
                    },
                ) = (&dims[..], &**elem)
                else {
                    return Err("an array that is not a one-dimensional logic vector".into());
                };
                self.whole_type(elem)?;
                let n = b.len();
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
                    bounds: Some(*b),
                    fresh: false,
                    leaf: None,
                    at: None,
                    obj: Some(d),
                }))
            }
            Shape::Record { fields, .. } => fields
                .iter()
                .map(|(_, f)| self.tree(f, d, leaf))
                .collect::<R<Vec<_>>>()
                .map(Val::Rec),
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

    fn static_int(&self, x: ExprId) -> R<i64> {
        let db = self.db;
        match db.exprs[x].value.map(|c| &db.consts[c]) {
            Some(ConstValue::Int(i)) => Ok(*i),
            _ => Err("an index that is not a literal or folded constant".into()),
        }
    }

    fn expr(&mut self, x: ExprId) -> R<Val> {
        let db = self.db;
        let ex = &db.exprs[x];
        if let Some(c) = ex.value {
            return self.literal(x, &db.consts[c]);
        }
        match &ex.kind {
            ExprKind::Obj(_)
            | ExprKind::Field { .. }
            | ExprKind::Index { .. }
            | ExprKind::Slice { .. } => match self.name(x, false)? {
                Val::Part(p) => Ok(Val::Part(self.materialise(p))),
                v => Ok(v),
            },
            // A qualified expression of a scalar logic type or an unconstrained array type checks
            // nothing a logic value of the operand's own type could fail.
            ExprKind::Qualified { to, operand }
                if matches!(
                    db.subtypes[*to].constraint,
                    yavs_sem::types::Constraint::None
                        | yavs_sem::types::Constraint::Array { dims: None, .. }
                ) =>
            {
                self.expr(*operand)
            }
            ExprKind::Conv { to, operand } => {
                // To an unconstrained logic array type: the same elements (LRM 9.3.6).
                let open = matches!(
                    db.subtypes[*to].constraint,
                    yavs_sem::types::Constraint::None
                        | yavs_sem::types::Constraint::Array { dims: None, .. }
                );
                let from = db.exprs[*operand].subtype;
                if !(open && self.logic_array(*to) && self.logic_array(from)) {
                    return Err(
                        "a type conversion that is not to an unconstrained logic vector type"
                            .into(),
                    );
                }
                self.expr(*operand)
            }
            ExprKind::Call {
                callee,
                args,
                method_of: None,
                implicit_cond: false,
            } => self.call(x, *callee, args),
            _ => Err("an expression that is not supported in cycle mode yet".into()),
        }
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

    /// The two-state bit of enumeration position `pos` of logic type `t` (BIT and BOOLEAN as
    /// '0'/'1'), and whether it is a metavalue read as 0. `'Z'` has no two-state value.
    fn bit(&self, t: yavs_sem::TypeId, pos: u32) -> R<(u64, bool)> {
        let wk = &self.db.well_known;
        if Some(t) == wk.bit || Some(t) == wk.boolean {
            return Ok((u64::from(pos), false));
        }
        if yavs_elab::elem_of(self.db, t) != yavs_kernel::Elem::Logic || pos >= 9 {
            return Err("a literal of a type that is not a logic type".into());
        }
        // U X 0 1 Z W L H -
        match pos {
            2 | 6 => Ok((0, false)),
            3 | 7 => Ok((1, false)),
            4 => Err("a `'Z'` literal: two-state values have no high impedance".into()),
            _ => Ok((0, true)),
        }
    }

    fn literal(&mut self, x: ExprId, c: &ConstValue) -> R<Val> {
        let db = self.db;
        let t = db.base(db.exprs[x].subtype);
        let bits: Vec<(u64, bool)> = match (c, &db.types[t].kind) {
            (ConstValue::Int(v), TypeKind::Integer | TypeKind::UniversalInteger) => {
                let i = self.tmp_int(*v, *v);
                self.ops.push(Op::IntLit { dst: i.r, v: *v });
                return Ok(Val::Int(i));
            }
            (ConstValue::Enum(p), _) => vec![self.bit(t, *p)?],
            (ConstValue::Array(a), TypeKind::Array { elem, .. }) => {
                let et = db.base(*elem);
                let Elems::Bytes(b) = &a.elems else {
                    return Err("a literal vector given in runs".into());
                };
                b.iter()
                    .map(|&p| self.bit(et, u32::from(p)))
                    .collect::<R<_>>()?
            }
            _ => return Err("a literal that is not a logic value or an integer".into()),
        };
        let n = bits.len() as u32;
        if n == 0 {
            return Err("a null literal".into());
        }
        if bits.iter().any(|b| b.1) {
            self.metas += 1;
        }
        // Row-major element `p` from the left is element `n - 1 - p`.
        let mut val = vec![0u64; words(n) as usize];
        for (p, &(b, _)) in bits.iter().enumerate() {
            let el = n as usize - 1 - p;
            val[el / 64] |= b << (el % 64);
        }
        let mut t = self.tmp(n);
        self.ops.push(Op::Lit {
            dst: t.r,
            val: val.into(),
        });
        if let ConstValue::Array(a) = c
            && let [(l, d, r)] = &a.bounds[..]
        {
            t.bounds = Some(Bounds {
                left: *l,
                dir: *d,
                right: *r,
            });
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

    fn call(&mut self, x: ExprId, callee: DeclId, args: &[yavs_sem::hir::Arg]) -> R<Val> {
        let db = self.db;
        let decl = &db.decls[callee];
        let Some(sp) = decl.kind.subprogram() else {
            return Err("a call of something that is not a subprogram".into());
        };
        use yavs_ir::ieee::{Form, Gate as G, Op as I};
        let gate = |g: G| match g {
            G::And => Gate::And,
            G::Or => Gate::Or,
            G::Xor => Gate::Xor,
            G::Nand => Gate::Nand,
            G::Nor => Gate::Nor,
            G::Xnor => Gate::Xnor,
        };
        enum K {
            Gate(Gate),
            Not,
            Cat,
            /// `numeric_std` `+`/`-` (`sub`) of unsigned vectors, the second operand an integer
            /// constant for `Form::Vi`.
            Plus(bool, Form),
            Eq(bool),
            Int(Arith),
            Cmp(Cmp),
        }
        let k = match &sp.body {
            yavs_sem::decl::Body::Implicit(op) => {
                let int = matches!(db.types[op.ty].kind, TypeKind::Integer);
                match op.family {
                    Family::And => K::Gate(Gate::And),
                    Family::Or => K::Gate(Gate::Or),
                    Family::Xor => K::Gate(Gate::Xor),
                    Family::Nand => K::Gate(Gate::Nand),
                    Family::Nor => K::Gate(Gate::Nor),
                    Family::Xnor => K::Gate(Gate::Xnor),
                    Family::Not => K::Not,
                    Family::Cat => K::Cat,
                    Family::Eq if int => K::Cmp(Cmp::Eq),
                    Family::Neq if int => K::Cmp(Cmp::Ne),
                    Family::Lt if int => K::Cmp(Cmp::Lt),
                    Family::Le if int => K::Cmp(Cmp::Le),
                    Family::Gt if int => K::Cmp(Cmp::Gt),
                    Family::Ge if int => K::Cmp(Cmp::Ge),
                    Family::Eq => K::Eq(false),
                    Family::Neq => K::Eq(true),
                    Family::Add if int => K::Int(Arith::Add),
                    Family::Sub if int => K::Int(Arith::Sub),
                    Family::Mul if int => K::Int(Arith::Mul),
                    Family::Div if int => K::Int(Arith::Div),
                    Family::Mod if int => K::Int(Arith::Mod),
                    Family::Rem if int => K::Int(Arith::Rem),
                    _ => {
                        return Err(format!(
                            "the predefined `{}` on these operands, which is not supported in \
                             cycle mode yet",
                            decl.name
                        ));
                    }
                }
            }
            _ => match yavs_ir::ieee::native_symbol(sp.mangled.as_str())
                .as_deref()
                .and_then(yavs_ir::ieee::symbol_key)
                .and_then(yavs_ir::ieee::parse)
            {
                Some(I::Gate(g) | I::VecGate(g) | I::NumGate(g)) => K::Gate(gate(g)),
                Some(I::Not | I::VecNot | I::NumNot) => K::Not,
                Some(I::Add {
                    sub,
                    signed: false,
                    form: form @ (Form::Vi | Form::Vv),
                }) => K::Plus(sub, form),
                _ => {
                    return Err(format!(
                        "a call of `{}`, which is not supported in cycle mode yet",
                        decl.name
                    ));
                }
            },
        };
        let two = |lx: &mut Self| -> R<(Val, Val)> {
            let [a, b] = args else {
                return Err(format!(
                    "a call of `{}` with an unexpected number of arguments",
                    decl.name
                ));
            };
            Ok((lx.arg(a)?, lx.arg(b)?))
        };
        match k {
            K::Gate(g) => {
                let (a, b) = two(self)?;
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
            K::Not => {
                let [a] = args else {
                    return Err("`not` with an unexpected number of arguments".into());
                };
                let a = self.arg(a)?;
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
            K::Cat => {
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
            K::Eq(ne) => {
                let (a, b) = two(self)?;
                let (a, b) = (self.part(a)?, self.part(b)?);
                let t = self.tmp(1);
                if a.len != b.len {
                    // Arrays of different lengths are never equal (LRM 9.2.3).
                    self.ops.push(Op::Lit {
                        dst: t.r,
                        val: Box::new([u64::from(ne)]),
                    });
                    return Ok(Val::Part(t));
                }
                let (ra, rb) = (self.whole(a), self.whole(b));
                self.ops.push(Op::VecEq {
                    dst: t.r,
                    a: ra,
                    b: rb,
                    len: a.len,
                    ne,
                });
                Ok(Val::Part(t))
            }
            K::Plus(sub, form) => {
                let [a, b] = args else {
                    return Err("`+` with an unexpected number of arguments".into());
                };
                let a = self.arg(a)?;
                let a = self.part(a)?;
                let rb = if form == Form::Vi {
                    let Actual::Expr(bx) = b.actual else {
                        return Err("a call with a default actual".into());
                    };
                    let k = match db.exprs[bx].value.map(|c| &db.consts[c]) {
                        Some(ConstValue::Int(k)) if *k >= 0 => *k as u64,
                        _ => {
                            return Err(
                                "`+` of an unsigned and an integer that is not a constant".into()
                            );
                        }
                    };
                    // A natural that does not fit warns in `to_unsigned`; not reproduced.
                    if a.len < 64 && k >> a.len != 0 {
                        return Err("`+` of a constant that does not fit the vector".into());
                    }
                    let t = self.tmp(a.len);
                    let mut val = vec![0u64; words(a.len) as usize];
                    val[0] = k;
                    self.ops.push(Op::Lit {
                        dst: t.r,
                        val: val.into(),
                    });
                    t.r
                } else {
                    let b = self.arg(b)?;
                    let b = self.part(b)?;
                    if b.len != a.len {
                        return Err(
                            "`+` of vectors of different lengths is not supported in cycle mode \
                             yet"
                            .into(),
                        );
                    }
                    self.whole(b)
                };
                let ra = self.whole(a);
                let t = self.tmp(a.len);
                self.ops.push(Op::Add {
                    dst: t.r,
                    a: ra,
                    b: rb,
                    len: a.len,
                    sub,
                });
                Ok(Val::Part(t))
            }
            K::Int(op) => {
                let (a, b) = two(self)?;
                let (a, b) = (self.int(a)?, self.int(b)?);
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
                Ok(Val::Int(t))
            }
            K::Cmp(op) => {
                let (a, b) = two(self)?;
                let (a, b) = (self.int(a)?, self.int(b)?);
                let t = self.tmp(1);
                self.ops.push(Op::IntCmp {
                    op,
                    dst: t.r,
                    a: a.r,
                    b: b.r,
                });
                Ok(Val::Part(t))
            }
        }
    }
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
