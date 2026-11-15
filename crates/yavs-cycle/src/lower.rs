//! The bodies of member processes, from the elaborated HIR to region operations. Milestone 1
//! keeps values in four planes (`yavs_logic::Block`): one block per logic leaf of at most 64
//! elements. Anything else is a reason to leave the process in the event kernel, never a guess.

use std::collections::{BTreeSet, HashMap};

use yavs_elab::{Bounds, Elaborated, LeafId, ProcInst, Shape};
use yavs_sem::hir::{Actual, ExprKind, Rhs, StmtKind, Waveform};
use yavs_sem::implicit::Family;
use yavs_sem::stat::{ConstValue, Elems};
use yavs_sem::types::{Direction, RangeC, TypeKind};
use yavs_sem::{Db, DeclId, ExprId, StmtId};

pub(crate) type Block = [u64; 4];

/// A block of the region's state: the current value of a kernel signal (all the leaves the
/// kernel keeps in it, each at its kernel offset), or a slot of the process (a leaf of a
/// variable, a temporary), numbered per body.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

/// One operation on blocks. Every operand is read before the result is written, so a result
/// may be an operand. Lengths are element counts of at most 64; bits above them stay 0.
#[derive(Clone, Debug)]
pub(crate) enum Op {
    Copy {
        dst: Ref,
        src: Ref,
    },
    Lit {
        dst: Ref,
        val: Block,
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
    /// Elements `lo .. lo + len` of `src` as a value of `len` elements.
    Extract {
        dst: Ref,
        src: Ref,
        lo: u32,
        len: u32,
    },
    /// `src`'s `len` elements into elements `lo .. lo + len` of `dst`.
    Insert {
        dst: Ref,
        src: Ref,
        lo: u32,
        len: u32,
    },
    /// `dst := dst & !mask | src & mask`, on every plane: the leaves of a record at the same
    /// places of two blocks, copied together.
    Blend {
        dst: Ref,
        src: Ref,
        mask: u64,
    },
    /// `hi & lo` (the left operand is the high part): `lo_len` elements of `lo` below `hi`.
    Cat {
        dst: Ref,
        hi: Ref,
        lo: Ref,
        lo_len: u32,
    },
    /// numeric_std `unsigned + natural` with a natural constant that fits `len` elements: all
    /// `X` when an element of `a` is not `0`, `1`, `L` or `H` (the body's `to_01`).
    AddK {
        dst: Ref,
        a: Ref,
        len: u32,
        k: u64,
    },
}

impl Op {
    pub(crate) fn dst(&self) -> Ref {
        match *self {
            Op::Copy { dst, .. }
            | Op::Lit { dst, .. }
            | Op::Gate { dst, .. }
            | Op::Not { dst, .. }
            | Op::Extract { dst, .. }
            | Op::Insert { dst, .. }
            | Op::Cat { dst, .. }
            | Op::Blend { dst, .. }
            | Op::AddK { dst, .. } => dst,
        }
    }

    fn dst_mut(&mut self) -> &mut Ref {
        match self {
            Op::Copy { dst, .. }
            | Op::Lit { dst, .. }
            | Op::Gate { dst, .. }
            | Op::Not { dst, .. }
            | Op::Extract { dst, .. }
            | Op::Insert { dst, .. }
            | Op::Cat { dst, .. }
            | Op::Blend { dst, .. }
            | Op::AddK { dst, .. } => dst,
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
    /// `Loc` slots used, `0 .. locs`.
    pub locs: u32,
    /// Asserts and reports left out (not evaluated in cycle mode).
    pub dropped: u32,
}

/// A value or a place: a part of one block, or a record of them.
#[derive(Clone, Debug)]
enum Val {
    Part(Part),
    Rec(Vec<Val>),
}

#[derive(Clone, Copy, Debug)]
struct Part {
    r: Ref,
    /// Elements `lo .. lo + len` of a block holding `full` elements.
    lo: u32,
    len: u32,
    full: u32,
    /// The index range of an array part (`None`: a scalar, or a value with no name).
    bounds: Option<Bounds>,
    /// A temporary made for this value alone: the operation that made it may write elsewhere.
    fresh: bool,
    /// The signal leaf the part is of (reads and writes of the body are counted by leaf).
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
        vars: HashMap::new(),
        ops: Vec::new(),
        reads: BTreeSet::new(),
        writes: BTreeSet::new(),
        locs: 0,
        dropped: 0,
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
        return Ok(Body {
            kind: Kind::Clocked,
            clock: Some(Clock {
                leaf: t.leaf,
                elem: t.elem,
                rising: t.rising,
                legacy: t.legacy,
            }),
            ops: lx.ops,
            reads: lx.reads,
            writes: lx.writes,
            locs: lx.locs,
            dropped: lx.dropped,
        });
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
    Ok(Body {
        kind: Kind::Comb,
        clock: None,
        ops: lx.ops,
        reads: lx.reads,
        writes: lx.writes,
        locs: lx.locs,
        dropped: lx.dropped,
    })
}

struct Lx<'a> {
    db: &'a Db,
    e: &'a Elaborated,
    p: &'a ProcInst,
    /// The leaves of each process variable met so far.
    vars: HashMap<DeclId, Val>,
    ops: Vec<Op>,
    reads: BTreeSet<LeafId>,
    writes: BTreeSet<LeafId>,
    locs: u32,
    dropped: u32,
}

impl Lx<'_> {
    fn loc(&mut self) -> Ref {
        self.locs += 1;
        Ref::Loc(self.locs - 1)
    }

    fn tmp(&mut self, len: u32) -> Part {
        Part {
            r: self.loc(),
            lo: 0,
            len,
            full: len,
            bounds: None,
            fresh: true,
            leaf: None,
        }
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
                let dst = self.name(target.expr, false)?;
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

    fn signal_rooted(&self, v: &Val) -> bool {
        match v {
            Val::Part(p) => matches!(p.r, Ref::Sig(_)),
            Val::Rec(fs) => fs.first().is_some_and(|f| self.signal_rooted(f)),
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
                let mut groups: Vec<(Ref, Ref, u64, u32)> = Vec::new();
                for (d, s) in pairs {
                    if s.fresh || d.lo != s.lo || d.len != s.len || d.full != s.full {
                        self.assign(&Val::Part(d), Val::Part(s))?;
                        continue;
                    }
                    if let Some(l) = d.leaf {
                        self.writes.insert(l);
                    }
                    let m = mask(d.len) << d.lo;
                    match groups.iter_mut().find(|g| (g.0, g.1) == (d.r, s.r)) {
                        Some(g) => g.2 |= m,
                        None => groups.push((d.r, s.r, m, d.full)),
                    }
                }
                for (dst, src, m, full) in groups {
                    if dst == src {
                        continue;
                    }
                    self.ops.push(if m == mask(full) {
                        Op::Copy { dst, src }
                    } else {
                        Op::Blend { dst, src, mask: m }
                    });
                }
                Ok(())
            }
            (Val::Part(d), Val::Part(s)) if d.len == s.len => {
                if let Some(l) = d.leaf {
                    self.writes.insert(l);
                }
                let whole = d.lo == 0 && d.len == d.full;
                // The operation that made a fresh temporary writes the target itself.
                if whole
                    && s.fresh
                    && s.lo == 0
                    && s.len == s.full
                    && let Some(op) = self.ops.last_mut().filter(|op| op.dst() == s.r)
                {
                    *op.dst_mut() = d.r;
                    return Ok(());
                }
                match (whole, s.lo == 0 && s.len == s.full) {
                    (true, true) if d.r == s.r => {}
                    (true, true) => self.ops.push(Op::Copy { dst: d.r, src: s.r }),
                    (true, false) => self.ops.push(Op::Extract {
                        dst: d.r,
                        src: s.r,
                        lo: s.lo,
                        len: s.len,
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
            _ => Err("an assignment whose value has another shape than its target".into()),
        }
    }

    /// The block holding exactly `p`'s elements.
    fn whole(&mut self, p: Part) -> Ref {
        if p.lo == 0 && p.len == p.full {
            return p.r;
        }
        let t = self.loc();
        self.ops.push(Op::Extract {
            dst: t,
            src: p.r,
            lo: p.lo,
            len: p.len,
        });
        t
    }

    fn part(&mut self, v: Val) -> R<Part> {
        match v {
            Val::Part(p) => Ok(p),
            Val::Rec(_) => Err("a record value as an operand".into()),
        }
    }

    /// A name: a signal, a variable of the process, or a field, element or slice of one.
    /// `target`: the name of a signal assignment target.
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
                let i = self.static_int(*i)?;
                let mut p = self.array_part(*prefix, target)?;
                let b = p.bounds.ok_or("an index of a value with no index range")?;
                let el = elem_of(b, i).ok_or("an index outside its array's range")?;
                p.lo += el;
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
            Val::Part(p) => Ok(p),
            Val::Rec(_) => Err("an index or slice of a record".into()),
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
                let v = self.tree(&so.shape, &mut |_| {
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
                let v = self.tree(shape, &mut |_| Ref::Loc(u32::MAX))?;
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

    /// Give a variable's leaves their slots.
    fn place_vars(&mut self, v: Val) -> Val {
        match v {
            Val::Part(mut p) => {
                p.r = self.loc();
                Val::Part(p)
            }
            Val::Rec(fs) => Val::Rec(fs.into_iter().map(|f| self.place_vars(f)).collect()),
        }
    }

    /// A signal's leaves (numbered `Loc(leaf)` by `tree`) at their kernel signal and offset.
    fn place_sig(&self, v: Val) -> R<Val> {
        match v {
            Val::Part(mut p) => {
                let Ref::Loc(l) = p.r else {
                    return Err("a signal leaf placed twice".into());
                };
                let l = LeafId(l);
                let nets = &self.e.nets;
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
                let full = nets.signal_len(at.signal);
                if full > 64 {
                    return Err(format!(
                        "the signal {}, kept in a kernel signal of more than 64 elements",
                        self.e.leaf_name(l)
                    ));
                }
                p.r = Ref::Sig(at.signal.0);
                p.lo = at.base;
                p.full = full;
                p.leaf = Some(l);
                Ok(Val::Part(p))
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
            Val::Part(Part { leaf: Some(l), .. }) => {
                self.reads.insert(*l);
            }
            Val::Part(_) => {}
            Val::Rec(fs) => fs.iter().for_each(|f| self.note_reads(f)),
        }
    }

    /// The leaves of `shape`: records of logic scalars and one-dimensional logic arrays of at
    /// most 64 elements.
    fn tree(&self, shape: &Shape, leaf: &mut dyn FnMut(u32) -> Ref) -> R<Val> {
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
                }))
            }
            Shape::Scalar { .. } => Err(
                "a value of an integer, enumeration or real type is not supported in cycle mode \
                 yet"
                .into(),
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
                if n == 0 || n > 64 {
                    return Err(format!(
                        "a vector of {n} elements (cycle mode takes 1 to 64 for now)"
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
                }))
            }
            Shape::Record { fields, .. } => fields
                .iter()
                .map(|(_, f)| self.tree(f, leaf))
                .collect::<R<Vec<_>>>()
                .map(Val::Rec),
        }
    }

    /// A logic scalar subtype that is its whole type: no assignment to it can fail a range
    /// check, which the region does not make (`X01`, `UX01` and the like are left out).
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
            | ExprKind::Slice { .. } => self.name(x, false),
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
            } => self.call(*callee, args),
            _ => Err("an expression that is not supported in cycle mode yet".into()),
        }
    }

    fn logic_array(&self, st: yavs_sem::SubtypeId) -> bool {
        let db = self.db;
        match &db.types[db.base(st)].kind {
            TypeKind::Array { indexes, elem } => {
                indexes.len() == 1
                    && yavs_elab::elem_of(db, db.base(*elem)) == yavs_kernel::Elem::Logic
            }
            _ => false,
        }
    }

    /// The code of enumeration position `pos` of logic type `t` (BIT and BOOLEAN as '0'/'1').
    fn code(&self, t: yavs_sem::TypeId, pos: u32) -> R<u8> {
        let wk = &self.db.well_known;
        if Some(t) == wk.bit || Some(t) == wk.boolean {
            return Ok(pos as u8 + 2);
        }
        if yavs_elab::elem_of(self.db, t) == yavs_kernel::Elem::Logic && pos < 9 {
            return Ok(pos as u8);
        }
        Err("a literal of a type that is not a logic type".into())
    }

    fn literal(&mut self, x: ExprId, c: &ConstValue) -> R<Val> {
        let db = self.db;
        let t = db.base(db.exprs[x].subtype);
        let codes: Vec<u8> = match (c, &db.types[t].kind) {
            (ConstValue::Enum(p), _) => vec![self.code(t, *p)?],
            (ConstValue::Array(a), TypeKind::Array { elem, .. }) => {
                let et = db.base(*elem);
                let Elems::Bytes(b) = &a.elems else {
                    return Err("a literal vector given in runs".into());
                };
                b.iter()
                    .map(|&p| self.code(et, u32::from(p)))
                    .collect::<R<_>>()?
            }
            _ => return Err("a literal that is not a logic value".into()),
        };
        let n = codes.len();
        if n == 0 || n > 64 {
            return Err(format!(
                "a literal of {n} elements (cycle mode takes 1 to 64 for now)"
            ));
        }
        // Row-major element `p` from the left is element `n - 1 - p`.
        let mut val = [0u64; 4];
        for (p, &code) in codes.iter().enumerate() {
            let el = n - 1 - p;
            for (k, plane) in val.iter_mut().enumerate() {
                *plane |= u64::from(code >> k & 1) << el;
            }
        }
        let mut t = self.tmp(n as u32);
        self.ops.push(Op::Lit { dst: t.r, val });
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

    fn arg(&mut self, a: &yavs_sem::hir::Arg) -> R<Part> {
        let Actual::Expr(x) = a.actual else {
            return Err("a call with a default or converted actual".into());
        };
        let v = self.expr(x)?;
        self.part(v)
    }

    fn call(&mut self, callee: DeclId, args: &[yavs_sem::hir::Arg]) -> R<Val> {
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
            Plus,
        }
        let k = match &sp.body {
            yavs_sem::decl::Body::Implicit(op) => match op.family {
                Family::And => K::Gate(Gate::And),
                Family::Or => K::Gate(Gate::Or),
                Family::Xor => K::Gate(Gate::Xor),
                Family::Nand => K::Gate(Gate::Nand),
                Family::Nor => K::Gate(Gate::Nor),
                Family::Xnor => K::Gate(Gate::Xnor),
                Family::Not => K::Not,
                Family::Cat => K::Cat,
                _ => {
                    return Err(format!(
                        "the predefined `{}`, which is not supported in cycle mode yet",
                        decl.name
                    ));
                }
            },
            _ => match yavs_ir::ieee::native_symbol(sp.mangled.as_str())
                .as_deref()
                .and_then(yavs_ir::ieee::symbol_key)
                .and_then(yavs_ir::ieee::parse)
            {
                Some(I::Gate(g) | I::VecGate(g) | I::NumGate(g)) => K::Gate(gate(g)),
                Some(I::Not | I::VecNot | I::NumNot) => K::Not,
                Some(I::Add {
                    sub: false,
                    signed: false,
                    form: Form::Vi,
                }) => K::Plus,
                _ => {
                    return Err(format!(
                        "a call of `{}`, which is not supported in cycle mode yet",
                        decl.name
                    ));
                }
            },
        };
        match (k, args) {
            (K::Gate(g), [a, b]) => {
                let (a, b) = (self.arg(a)?, self.arg(b)?);
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
            (K::Not, [a]) => {
                let a = self.arg(a)?;
                let ra = self.whole(a);
                let t = self.tmp(a.len);
                self.ops.push(Op::Not {
                    dst: t.r,
                    a: ra,
                    len: a.len,
                });
                Ok(Val::Part(t))
            }
            (K::Cat, [a, b]) => {
                let (a, b) = (self.arg(a)?, self.arg(b)?);
                let n = a.len + b.len;
                if n > 64 {
                    return Err(format!(
                        "a concatenation of {n} elements (cycle mode takes up to 64 for now)"
                    ));
                }
                let (ra, rb) = (self.whole(a), self.whole(b));
                let t = self.tmp(n);
                self.ops.push(Op::Cat {
                    dst: t.r,
                    hi: ra,
                    lo: rb,
                    lo_len: b.len,
                });
                Ok(Val::Part(t))
            }
            (K::Plus, [a, b]) => {
                let a = self.arg(a)?;
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
                // A natural that does not fit warns in `to_unsigned`; not reproduced yet.
                if a.len < 64 && k >> a.len != 0 {
                    return Err("`+` of a constant that does not fit the vector".into());
                }
                let ra = self.whole(a);
                let t = self.tmp(a.len);
                self.ops.push(Op::AddK {
                    dst: t.r,
                    a: ra,
                    len: a.len,
                    k,
                });
                Ok(Val::Part(t))
            }
            _ => Err(format!(
                "a call of `{}` with an unexpected number of arguments",
                decl.name
            )),
        }
    }
}

/// The leaves of two records of one shape, pairwise.
fn flatten(d: &Val, s: Val, out: &mut Vec<(Part, Part)>) -> R<()> {
    match (d, s) {
        (Val::Rec(ds), Val::Rec(ss)) if ds.len() == ss.len() => {
            for (d, s) in ds.iter().zip(ss) {
                flatten(d, s, out)?;
            }
            Ok(())
        }
        (Val::Part(d), Val::Part(s)) if d.len == s.len => {
            out.push((*d, s));
            Ok(())
        }
        _ => Err("an assignment whose value has another shape than its target".into()),
    }
}

fn mask(len: u32) -> u64 {
    if len >= 64 { !0 } else { (1u64 << len) - 1 }
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
