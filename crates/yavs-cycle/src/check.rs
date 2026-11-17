//! The subset checker: whether a process can run cycle-based at all, independent of what the
//! lowering supports yet. It reads the elaborated HIR of one process instance (generics folded,
//! loop parameters of static `for` loops enumerated) and answers with the process's shape and
//! its combinational dependences ([`Fit`]), a behavioural process ([`Verdict::Behavioural`]), or
//! the reason it is outside the subset ([`Reason`], with a stable code).
//!
//! Soundness by direction: the elements a value may depend on are over-approximated (a dynamic
//! index reads the whole object, a call depends on all its arguments), the elements assigned on
//! every path are under-approximated (a dynamic index or an unbounded loop assigns nothing), and
//! the wait set of a process is under-approximated. Every imprecision leaves a process out; none
//! can let a latch, a state variable or an incomplete sensitivity list in.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;

use yavs_diag::Span;
use yavs_elab::{Elaborated, LeafId, NetGraph, ObjId, ProcInst, Sel, Shape};
use yavs_kernel::Elem;
use yavs_sem::decl::{Alias, Body, DeclKind, ObjClass, ObjRole, SubprogramKind};
use yavs_sem::hir::{
    Actual, AttrPrefix, ExprKind, LoopScheme, Mode, PredefAttr, Rhs, Sensitivity, StmtKind,
    Waveform,
};
use yavs_sem::implicit::Family;
use yavs_sem::stat::{ConstValue, Elems};
use yavs_sem::types::{Constraint, Direction, RangeC, TypeKind};
use yavs_sem::{Db, DeclId, ExprId, Id, StmtId, SubtypeId};

use crate::codes;
use crate::dep::{Dep, ESet, Out, VAR};

/// The shape of a process that fits the subset.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Form {
    Clocked,
    Comb,
}

/// A process inside the subset.
#[derive(Clone)]
pub(crate) struct Fit {
    pub form: Form,
    /// The clock: leaf, element, rising.
    pub clock: Option<(LeafId, u32, bool)>,
    /// The signals an asynchronous reset reads.
    pub reset: Vec<LeafId>,
    /// Its combinational dependences: of every driven run of a combinational process, of what the
    /// reset branch of a clocked one assigns.
    pub outs: Vec<Out>,
    /// Asserts and reports, not evaluated in cycle mode.
    pub dropped: u32,
    /// Every signal element it reads (over-approximated).
    pub reads: ESet,
}

/// Why a process stays out: its code, the statement that causes it, the text, and the other
/// side of a situation that spans processes (another process or signal, where it is). The text
/// is `construct: detail`: the construct a noun phrase ("<construct> is not supported in cycle
/// mode"), the optional detail what at the statement causes it.
#[derive(Clone, Debug)]
pub(crate) struct Reason {
    pub code: &'static str,
    pub span: Span,
    pub text: String,
    pub also: Option<(Span, String)>,
}

impl Reason {
    pub(crate) fn new(code: &'static str, span: Span, text: impl Into<String>) -> Self {
        Reason {
            code,
            span,
            text: text.into(),
            also: None,
        }
    }

    pub(crate) fn also(mut self, span: Span, what: impl Into<String>) -> Self {
        self.also = Some((span, what.into()));
        self
    }
}

pub(crate) enum Verdict {
    Fit(Fit),
    /// A process of the testbench kind, which stays in the event kernel by nature.
    Behavioural(String),
    Out(Reason),
}

enum Stop {
    Out(Reason),
    Behav(String),
    /// The statement budget of static loop enumeration ran out.
    Budget,
}

type R<T> = Result<T, Stop>;

/// Runs of elements: key, first, end.
type Parts = Vec<(u64, u32, u32)>;

fn out<T>(code: &'static str, span: Span, text: impl Into<String>) -> R<T> {
    Err(Stop::Out(Reason::new(code, span, text)))
}

/// A reason no design reaches (analysis or an earlier check rules it out): an internal error,
/// a panic in debug builds.
fn bug<T>(span: Span, what: impl Into<String>) -> R<T> {
    Err(Stop::Out(internal(span, what.into())))
}

fn internal(span: Span, what: String) -> Reason {
    debug_assert!(false, "cycle mode: {what}");
    Reason::new(
        codes::INTERNAL,
        span,
        format!("{}{what}", crate::lower::BUG),
    )
}

/// Statement instances a process may take while static loops are enumerated; past that, loops
/// are analysed without their parameter's value.
const BUDGET: usize = 1 << 16;
/// Rounds of a loop analysed without its parameter's value before the analysis gives up.
const ROUNDS: usize = 24;

/// The value of expression `x` of process `p` when it folds: constants, `known` (the values of
/// names the caller knows: loop parameters, formals), and pure functions of them run by the
/// analysis's evaluator. A call that reaches an `assert` or `report` does not fold here: the
/// lowering leaves its reports to elaboration or keeps the process out (A22).
pub(crate) fn const_value(
    db: &Db,
    e: &Elaborated,
    p: &ProcInst,
    known: HashMap<DeclId, ConstValue>,
    consts: &mut Consts,
    x: ExprId,
) -> Option<ConstValue> {
    let mut cx = Cx::new(db, e, p);
    cx.loopv = known;
    cx.quiet_only = true;
    cx.consts = std::mem::take(consts);
    let v = cx.fold(x);
    *consts = std::mem::take(&mut cx.consts);
    v
}

/// Constants of elaboration folded as a name of the process's scope gives them (with their
/// subtype's index ranges at every level, `bounded`), shared within one process's check or
/// lowering: an element or field read is taken from here instead of copying and rebounding the
/// whole constant at every read.
pub(crate) type Consts = HashMap<(yavs_elab::ScopeId, DeclId, SubtypeId), Option<Rc<ConstValue>>>;

/// The element of constant array `a` at indices `ix`, one per dimension (the elements stored
/// row by row, the last index fastest); `None` outside the bounds.
pub(crate) fn const_elem(a: &yavs_sem::stat::ArrayConst, ix: &[i64]) -> Option<ConstValue> {
    if ix.len() != a.bounds.len() {
        return None;
    }
    let mut p: u64 = 0;
    for (&i, &(l, d, r)) in ix.iter().zip(&a.bounds[..]) {
        let (k, n) = match d {
            Direction::To if i >= l && i <= r => (i - l, r - l + 1),
            Direction::Downto if i <= l && i >= r => (l - i, l - r + 1),
            _ => return None,
        };
        p = p
            .checked_mul(u64::try_from(n).ok()?)?
            .checked_add(u64::try_from(k).ok()?)?;
    }
    match &a.elems {
        Elems::Bytes(b) => b.get(p as usize).map(|&v| ConstValue::Enum(u32::from(v))),
        Elems::Runs(runs) => {
            let mut at = 0u64;
            runs.iter().find_map(|(n, v)| {
                at += n;
                (p < at).then(|| v.clone())
            })
        }
    }
}

/// Folded value `v` of subtype `st` with that subtype's index ranges at every level: a folded value
/// carries them only at the top, an element keeps its literal's (`x"01"` is `0 to 7` in an array
/// of `std_ulogic_vector(7 downto 0)`). `None` when a length differs or a range below the top is not
/// known here; one at the top (of the value and of its record fields) is the value's, as
/// elaboration gave a constant its subtype's there (LRM 5.3.2.2).
pub(crate) fn bounded(
    db: &Db,
    v: ConstValue,
    st: SubtypeId,
    range: &mut dyn FnMut(&RangeC) -> Option<yavs_elab::Bounds>,
) -> Option<ConstValue> {
    bounded_in(db, v, db.base(st), &db.subtypes[st].constraint, true, range)
}

/// `bounded` for a value of type `t` whose own index ranges at the top are kept.
pub(crate) fn bounded_type(
    db: &Db,
    v: ConstValue,
    t: yavs_sem::TypeId,
    range: &mut dyn FnMut(&RangeC) -> Option<yavs_elab::Bounds>,
) -> Option<ConstValue> {
    bounded_in(db, v, t, &Constraint::None, true, range)
}

fn bounded_in(
    db: &Db,
    v: ConstValue,
    t: yavs_sem::TypeId,
    c: &Constraint,
    top: bool,
    range: &mut dyn FnMut(&RangeC) -> Option<yavs_elab::Bounds>,
) -> Option<ConstValue> {
    let own = |c: &Constraint| match c {
        Constraint::Array { dims: Some(d), .. } => Some(d.clone()),
        _ => None,
    };
    match (v, &db.types[t].kind) {
        (ConstValue::Array(mut a), TypeKind::Array { elem, .. }) => {
            if let Some(dims) =
                own(c).or_else(|| own(&db.subtypes[db.types[t].base_subtype].constraint))
            {
                let want = (dims.iter())
                    .map(|r| range(r).map(|b| (b.left, b.dir, b.right)))
                    .collect::<Option<Vec<_>>>();
                match want {
                    Some(w) if *a.bounds != *w => a = a.with_bounds(&w)?,
                    Some(_) => {}
                    None if top => {}
                    None => return None,
                }
            }
            // The element constraint of the object's subtype, else the element subtype's own.
            let ec = match c {
                Constraint::Array { elem: ec, .. } if !matches!(**ec, Constraint::None) => ec,
                _ => &db.subtypes[*elem].constraint,
            };
            let et = db.base(*elem);
            if let Elems::Runs(runs) = &mut a.elems
                && !db.types[et].kind.is_scalar()
            {
                *runs = (std::mem::take(runs).into_vec().into_iter())
                    .map(|(n, x)| Some((n, bounded_in(db, x, et, ec, false, range)?)))
                    .collect::<Option<_>>()?;
            }
            Some(ConstValue::Array(a))
        }
        (ConstValue::Record(fs), TypeKind::Record { elems }) if fs.len() == elems.len() => {
            let fs = (fs.into_vec().into_iter().zip(elems.iter()).enumerate())
                .map(|(i, (f, el))| {
                    let fc = match c {
                        Constraint::Record { elems: rc } => {
                            rc.iter().find(|(j, _)| *j as usize == i).map(|(_, fc)| fc)
                        }
                        _ => None,
                    };
                    let fc = fc.unwrap_or(&db.subtypes[el.subtype].constraint);
                    bounded_in(db, f, db.base(el.subtype), fc, top, range)
                })
                .collect::<Option<_>>()?;
            Some(ConstValue::Record(fs))
        }
        (v, _) => Some(v),
    }
}

/// An expression whose folded value is an object's or a call's, so of its subtype: `bounded`
/// gives it that subtype's index ranges.
/// Whether every value of the type of subtype `st` lies in it: an array of no index
/// constraint, an enumeration of all its literals. Elaboration's evaluation of a call checks no
/// argument against its formal, so the folds take its value only when each fits (`call_fits`).
pub(crate) fn takes_any(db: &Db, st: SubtypeId) -> bool {
    match &db.types[db.base(st)].kind {
        TypeKind::Array { .. } => matches!(
            db.subtypes[st].constraint,
            Constraint::None | Constraint::Array { dims: None, .. }
        ),
        TypeKind::Enum { lits } => db.static_bounds(st).is_some_and(|(l, _, r)| {
            l.as_int() == Some(0) && r.as_int() == Some(lits.len() as i64 - 1)
        }),
        _ => false,
    }
}

/// Scalar `v` lies in subtype `st` (its static range).
fn scalar_fits(db: &Db, st: SubtypeId, v: &ConstValue) -> bool {
    let (Some((l, dir, r)), Some(v)) = (db.static_bounds(st), v.as_int()) else {
        return false;
    };
    let (Some(l), Some(r)) = (l.as_int(), r.as_int()) else {
        return false;
    };
    let (lo, hi) = if dir == Direction::Downto {
        (r, l)
    } else {
        (l, r)
    };
    (lo..=hi).contains(&v)
}

pub(crate) fn named(k: &ExprKind) -> bool {
    matches!(
        k,
        ExprKind::Obj(_)
            | ExprKind::Index { .. }
            | ExprKind::Field { .. }
            | ExprKind::Call { .. }
            | ExprKind::Qualified { .. }
    )
}

/// What the subprograms of a design do as far as a process cares, kept from one process's check
/// to the next.
pub(crate) type Callees = HashMap<DeclId, Callee>;

/// Check process `pi`.
pub(crate) fn check(db: &Db, e: &Elaborated, pi: usize, callees: &mut Callees) -> Verdict {
    let p = &e.procs[pi];
    let mut cx = Cx::new(db, e, p);
    cx.callees = std::mem::take(callees);
    let v = cx.process();
    *callees = std::mem::take(&mut cx.callees);
    match v {
        Ok(mut f) => {
            f.reads = std::mem::take(&mut cx.reads);
            // The clock is read too (its edge test adds no dependence).
            if let Some((l, el, _)) = f.clock {
                f.reads.add(u64::from(l.0), el, el + 1);
            }
            Verdict::Fit(f)
        }
        Err(Stop::Out(r)) => Verdict::Out(r),
        Err(Stop::Behav(w)) => Verdict::Behavioural(w),
        // The outermost enumerated loop takes it and goes on without the parameter's value.
        Err(Stop::Budget) => Verdict::Out(internal(
            db.stmts[p.stmt].span,
            "the loop budget ran out outside an enumerated loop".into(),
        )),
    }
}

/// How a statement list of a pure function run on constants ends.
enum Pure {
    Fall,
    Return(ConstValue),
    Exit(StmtId),
    Next(StmtId),
    /// A value or statement the evaluation does not know: the call is not folded.
    GiveUp,
}

/// A scalar value: the evaluator of pure functions keeps no composites, whose bounds would have
/// to be the object's subtype's.
fn scalar(v: &ConstValue) -> bool {
    matches!(
        v,
        ConstValue::Int(_) | ConstValue::Enum(_) | ConstValue::Phys(_)
    )
}

/// A record aggregate of scalar values that `fold` knows (positional, named, `others`), as a
/// record of scalars, and the predefined `=` and `/=` of two such records; `None` for any other
/// expression.
pub(crate) fn record_fold(
    db: &Db,
    x: ExprId,
    fold: &mut dyn FnMut(ExprId) -> Option<ConstValue>,
) -> Option<ConstValue> {
    use yavs_sem::hir::{AggBounds, Choice};
    if let ExprKind::Call { callee, args, .. } = &db.exprs[x].kind
        && db.decls[*callee].implicit
        && let [a, b] = &args[..]
        && let (Actual::Expr(a), Actual::Expr(b)) = (&a.actual, &b.actual)
    {
        let ne = match db.decls[*callee].name.key().as_str() {
            "\"=\"" => false,
            "\"/=\"" => true,
            _ => return None,
        };
        let (a, b) = (
            fold(*a).filter(scalar_record)?,
            fold(*b).filter(scalar_record)?,
        );
        return Some(ConstValue::Enum(u32::from((a == b) != ne)));
    }
    let ExprKind::Aggregate {
        assocs,
        bounds: AggBounds::Record,
    } = &db.exprs[x].kind
    else {
        return None;
    };
    let TypeKind::Record { elems } = &db.types[db.base(db.exprs[x].subtype)].kind else {
        return None;
    };
    let mut out: Vec<Option<ConstValue>> = vec![None; elems.len()];
    for (k, a) in assocs.iter().enumerate() {
        let at: Vec<usize> = if a.choices.is_empty() {
            vec![k]
        } else if a.choices.iter().any(|c| matches!(c, Choice::Others)) {
            (0..elems.len()).filter(|&f| out[f].is_none()).collect()
        } else {
            (a.choices.iter())
                .map(|c| match c {
                    Choice::Element(f) => Some(*f as usize),
                    _ => None,
                })
                .collect::<Option<_>>()?
        };
        for f in at {
            // A field outside its subtype is left to the code, which fails as the normal mode does.
            let v = fold(a.value).filter(scalar)?;
            let (l, dir, r) = db.static_bounds(elems.get(f)?.subtype)?;
            let (l, r, x) = (l.as_int()?, r.as_int()?, v.as_int()?);
            let (lo, hi) = if dir == Direction::Downto {
                (r, l)
            } else {
                (l, r)
            };
            if !(lo..=hi).contains(&x) {
                return None;
            }
            *out.get_mut(f)? = Some(v);
        }
    }
    out.into_iter()
        .collect::<Option<Vec<_>>>()
        .map(|fs| ConstValue::Record(fs.into()))
}

/// A record whose fields are all scalars (a fixed-point format `(s, i, f)`).
pub(crate) fn scalar_record(v: &ConstValue) -> bool {
    matches!(v, ConstValue::Record(fs) if fs.iter().all(scalar))
}

/// What a call does, as far as a process cares (memoised per subprogram).
#[derive(Clone, Default)]
pub(crate) struct Callee {
    /// A procedure that may wait.
    waits: bool,
    /// It uses files, text input and output or access values.
    files: bool,
    /// Calls into code the analysis cannot see.
    opaque: Option<String>,
    /// Changes something outside it (a variable or signal it is not given, a shared variable).
    writes_outside: Option<String>,
    /// Signals it reads that are not its parameters.
    sig_reads: Vec<DeclId>,
    /// Per parameter: an `out` one assigned as a whole by a statement every call runs.
    must_out: Vec<bool>,
    /// It calls `std.env` or another simulator procedure.
    control: bool,
    /// What it does that no process of the region may: the code and what.
    bad: Option<(&'static str, String)>,
}

/// The analysis state at one point of a process.
#[derive(Clone, Default, PartialEq)]
struct St {
    /// What each assigned element (a variable's, or a signal's pending value) depends on, as
    /// runs `lo .. hi` per key.
    vals: BTreeMap<u64, Vec<(u32, u32, Dep)>>,
    /// Elements assigned on every path to here.
    def: ESet,
    /// What reaching here depends on (conditions of enclosing statements and earlier exits).
    ctl: Rc<ESet>,
    /// The values of the process's own scalar variables assigned a value that folds, on every
    /// path to here (`Cx::vtrack`).
    known: Rc<HashMap<DeclId, ConstValue>>,
}

/// Where control goes after a statement list.
#[derive(Default)]
struct Flow {
    fall: Option<St>,
    exits: Vec<(StmtId, St)>,
    nexts: Vec<(StmtId, St)>,
}

impl Flow {
    fn fall(s: St) -> Flow {
        Flow {
            fall: Some(s),
            ..Flow::default()
        }
    }
}

/// A value's dependences: runs in element order, or one set for all elements.
enum Dv {
    Flat(Rc<ESet>),
    Runs(Vec<(u32, Dep)>),
}

/// A name resolved to elements.
struct Place {
    /// Signal leaves, or variable leaves with `VAR` set.
    parts: Vec<(u64, u32, u32)>,
    /// The parts are exactly the elements named (no dynamic index, no rebased alias).
    precise: bool,
    sig: bool,
    /// What the index and slice expressions read.
    idx: ESet,
    decl: Option<DeclId>,
}

enum Root {
    Sig(ObjId),
    Var(DeclId),
    /// A constant, generic, loop parameter or literal: no dependence.
    Const,
}

struct Name {
    root: Root,
    decl: Option<DeclId>,
    sels: Vec<Sel>,
    stopped: bool,
    idx: ESet,
}

struct Cx<'a> {
    db: &'a Db,
    e: &'a Elaborated,
    p: &'a ProcInst,
    /// Combinational analysis (latches, state variables) is on: a combinational process or the
    /// reset branch of a clocked one.
    comb: bool,
    /// The clock element: an edge test of it is the trigger, not a history read.
    clock: Option<(LeafId, u32)>,
    /// The layout of the process's variables and of ports collapsed into parts of leaves.
    vars: NetGraph,
    var_obj: HashMap<DeclId, Option<ObjId>>,
    part_obj: HashMap<ObjId, ObjId>,
    /// Values of loop parameters being enumerated.
    loopv: HashMap<DeclId, ConstValue>,
    /// The scalar variables of the process that only its own plain assignments write (no
    /// procedure is given one, no subprogram of the process may write one), and the values of
    /// those known before the statement being analysed (`St::known`): `lo := hi + 1` with `hi`
    /// known makes `v(hi downto lo)` a slice of known bounds.
    vtrack: HashSet<DeclId>,
    vknown: Rc<HashMap<DeclId, ConstValue>>,
    /// Parameters and variables of pure functions being run on constants.
    frames: Vec<HashMap<DeclId, ConstValue>>,
    /// Variable elements the process may write.
    var_any: ESet,
    /// Variable elements read before they were assigned on some path, where, and the variable.
    undef_reads: Vec<(ESet, Span, Option<DeclId>)>,
    /// Signal elements read.
    reads: ESet,
    /// Signal elements any path may assign.
    sig_any: ESet,
    /// Where each signal leaf is first read (for messages).
    read_at: HashMap<u64, Span>,
    dropped: u32,
    /// A process of the top-level unit: its asserts and reports are evaluated (A19), so what
    /// they read counts.
    top: bool,
    budget: usize,
    enumerating: u32,
    /// Elements a join of branches lost from the definitely assigned ones, and where.
    lost: Vec<(ESet, Span)>,
    callees: HashMap<DeclId, Callee>,
    consts: Consts,
    /// The subprograms being summarised, by their depth in that walk, and the smallest depth of
    /// one of them called again under the current walk (`usize::MAX`: none).
    visiting: HashMap<DeclId, usize>,
    partial: usize,
    /// `const_value`: a pure function run on constants gives up at an `assert` or `report`.
    quiet_only: bool,
    /// Constants of the pure functions being run whose values did not fold.
    opaque: std::collections::HashSet<DeclId>,
}

impl<'a> Cx<'a> {
    fn new(db: &'a Db, e: &'a Elaborated, p: &'a ProcInst) -> Cx<'a> {
        Cx {
            db,
            e,
            p,
            comb: false,
            clock: None,
            vars: NetGraph::default(),
            var_obj: HashMap::new(),
            part_obj: HashMap::new(),
            loopv: HashMap::new(),
            vtrack: HashSet::new(),
            vknown: Rc::default(),
            frames: Vec::new(),
            var_any: ESet::default(),
            undef_reads: Vec::new(),
            reads: ESet::default(),
            sig_any: ESet::default(),
            read_at: HashMap::new(),
            dropped: 0,
            top: crate::region::in_top_scope(e, p.scope),
            budget: BUDGET,
            enumerating: 0,
            lost: Vec::new(),
            callees: HashMap::new(),
            consts: HashMap::new(),
            visiting: HashMap::new(),
            partial: usize::MAX,
            quiet_only: false,
            opaque: Default::default(),
        }
    }
}

impl Cx<'_> {
    fn span(&self) -> Span {
        self.db.stmts[self.p.stmt].span
    }

    fn process(&mut self) -> R<Fit> {
        let (db, p) = (self.db, self.p);
        if p.postponed {
            return out(codes::GUARDED, self.span(), "a postponed process");
        }
        if p.guard.is_some() {
            return out(codes::GUARDED, self.span(), "a guarded block");
        }
        if let Some(pa) = &p.actual {
            // `formal <= expr`: a combinational process with the expression's reads as its list.
            self.comb = true;
            let mut st = St::default();
            let v = self.expr(pa.expr, &st)?;
            let v = self.flat(&v);
            let mut wait = ESet::default();
            let mut lsps = Vec::new();
            db.reads(pa.expr, &mut lsps);
            self.lsps_into(&lsps, &mut wait);
            for (l, r) in self.driven() {
                st.def.add(l, r.0, r.1);
                st.vals
                    .insert(l, vec![(r.0, r.1, Dep::set(Rc::new(v.clone())))]);
            }
            return self.finish_comb(st, wait);
        }
        let s = &db.stmts[p.stmt];
        match &s.kind {
            StmtKind::Process {
                sensitivity,
                body,
                scope,
                ..
            } => {
                if matches!(sensitivity, Sensitivity::None) {
                    self.waits_behavioural(body)?;
                }
                self.decls(*scope)?;
                self.uses_files(body)?;
                self.vtrack = tracked_vars(db, *scope, body);
                self.process_body(sensitivity, body)
            }
            StmtKind::ConcSignalAssign {
                guarded,
                target,
                delay,
                rhs,
                ..
            } => {
                if *guarded {
                    return out(codes::GUARDED, s.span, "a guarded assignment");
                }
                // `q <= d when <edge>;` is a clocked process.
                if let Rhs::Conditional(alts) = rhs
                    && let [(_, Some(c))] = &alts[..]
                    && let Some((edge, extras)) = self.edge_head(*c)
                {
                    let (leaf, el) = self.clock_elem(edge.sig)?;
                    self.clock = Some((leaf, el));
                    let st = St::default();
                    for x in extras {
                        self.expr(x, &st)?;
                    }
                    let mut st = St::default();
                    if let Rhs::Conditional(alts) = rhs {
                        self.signal_assign_value(&mut st, target.expr, delay, &alts[0].0, s.span)?;
                    }
                    return Ok(Fit {
                        form: Form::Clocked,
                        clock: Some((leaf, el, edge.rising)),
                        reset: Vec::new(),
                        outs: Vec::new(),
                        dropped: 0,
                        reads: ESet::default(),
                    });
                }
                self.comb = true;
                let st = St::default();
                let flow = self.signal_assign(st, target.expr, delay, rhs, s.span)?;
                let st = flow.fall.unwrap_or_default();
                let mut wait = ESet::default();
                let mut lsps = Vec::new();
                db.stmt_reads(p.stmt, &mut lsps);
                self.lsps_into(&lsps, &mut wait);
                self.finish_comb(st, wait)
            }
            StmtKind::ConcAssert { .. } => Ok(Fit {
                form: Form::Comb,
                clock: None,
                reset: Vec::new(),
                outs: Vec::new(),
                dropped: 1,
                reads: ESet::default(),
            }),
            StmtKind::ConcProcCall { call, .. } => {
                let c = self.callee(call.callee);
                if c.waits {
                    return Err(Stop::Behav(format!(
                        "it calls `{}`, which can wait",
                        db.decls[call.callee].name
                    )));
                }
                self.comb = true;
                let st = St::default();
                let st = self.proc_call(st, call.callee, &call.args, call.span)?;
                let mut wait = ESet::default();
                let mut lsps = Vec::new();
                db.stmt_reads(p.stmt, &mut lsps);
                self.lsps_into(&lsps, &mut wait);
                self.finish_comb(st, wait)
            }
            // Elaboration makes processes of these four kinds of statements only.
            _ => bug(
                s.span,
                "a process of a concurrent statement of another kind",
            ),
        }
    }

    /// The process's own declarations: files and access values make it behavioural; real,
    /// time, protected objects are outside the subset.
    fn decls(&mut self, scope: yavs_sem::ScopeId) -> R<()> {
        let db = self.db;
        for &d in &db.scopes[scope].order {
            let decl = &db.decls[d];
            let DeclKind::Object(o) = &decl.kind else {
                continue;
            };
            if o.class == ObjClass::File {
                return Err(Stop::Behav(format!("it declares the file `{}`", decl.name)));
            }
            match &db.types[db.base(o.subtype)].kind {
                TypeKind::Access { .. } => {
                    return Err(Stop::Behav(format!(
                        "it declares `{}`, an access value (text lines and the like)",
                        decl.name
                    )));
                }
                TypeKind::Protected { .. } => {
                    return Err(Stop::Behav(format!(
                        "it declares `{}`, a protected object (a random generator and the like)",
                        decl.name
                    )));
                }
                TypeKind::Floating | TypeKind::Physical { .. } if o.class != ObjClass::Constant => {
                    return out(
                        codes::NOT_TWO_STATE,
                        decl.span,
                        format!("the real or physical (time) object `{}`", decl.name),
                    );
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Files, text input and output, `std.env` anywhere in the process make it behavioural.
    fn uses_files(&mut self, body: &[StmtId]) -> R<()> {
        let db = self.db;
        let mut calls = Vec::new();
        let mut stmts = Vec::new();
        deep(db, body, &mut stmts);
        for &s in &stmts {
            if matches!(
                db.stmts[s].kind,
                StmtKind::Assert { .. } | StmtKind::Report { .. }
            ) {
                continue;
            }
            if let StmtKind::ProcCall(c) = &db.stmts[s].kind {
                calls.push(c.callee);
            }
            db.stmt_exprs(s, &mut |x, _| {
                db.walk_expr(x, &mut |y| {
                    if let ExprKind::Call { callee, .. } = &db.exprs[y].kind {
                        calls.push(*callee);
                    }
                })
            });
        }
        for c in calls {
            let info = self.callee(c);
            if info.files {
                return Err(Stop::Behav(format!(
                    "it calls `{}`, which uses files or text input and output",
                    db.decls[c].name
                )));
            }
            if info.control {
                return Err(Stop::Behav(format!(
                    "it calls `{}`, which controls the simulation",
                    db.decls[c].name
                )));
            }
        }
        Ok(())
    }

    fn process_body(&mut self, sens: &Sensitivity, body: &[StmtId]) -> R<Fit> {
        let db = self.db;
        // Asserts and reports at the top level are not evaluated, so they do not shape it.
        let kept: Vec<StmtId> = (body.iter().copied())
            .filter(|&s| {
                let msg = matches!(
                    db.stmts[s].kind,
                    StmtKind::Assert { .. } | StmtKind::Report { .. }
                );
                self.dropped += u32::from(msg);
                !msg
            })
            .collect();
        // Those of the top-level unit are evaluated (A19): what they read counts, as in `stmt`.
        if self.top {
            let mut lsps = Vec::new();
            for &s in body.iter().filter(|s| !kept.contains(s)) {
                db.stmt_exprs(s, &mut |x, _| db.reads(x, &mut lsps));
            }
            let mut reads = std::mem::take(&mut self.reads);
            self.lsps_into(&lsps, &mut reads);
            self.reads = reads;
        }
        if matches!(sens, Sensitivity::None) {
            return self.wait_form(&kept);
        }
        let wait = self.wait_set(sens);
        // `if E then B end if;` or `if R then A elsif E then B end if;`.
        if let [only] = &kept[..]
            && let StmtKind::If { branches, else_ } = &db.stmts[*only].kind
        {
            let heads: Vec<Option<_>> = branches.iter().map(|b| self.edge_head(b.0)).collect();
            let shape = match (&branches[..], &heads[..], else_) {
                ([(_, b)], [Some(h)], None) => Some((None, b, h.clone())),
                ([(r, a), (_, b)], [None, Some(h)], None) => {
                    Some((Some((*r, &a[..])), b, h.clone()))
                }
                _ => None,
            };
            if let Some((reset, inner, (edge, extras))) = shape {
                return self.clocked(&wait, reset, inner, edge, extras, db.stmts[*only].span);
            }
        }
        // Not a clocked shape: an edge test anywhere is one cycle mode does not take.
        if let Some(x) = self.find_edge(body) {
            return out(
                codes::SHAPE,
                db.exprs[x].span,
                "a clocked process of another shape: an edge test outside the clock-edge template",
            );
        }
        self.comb = true;
        let flow = self.stmts(body, St::default())?;
        let st = self.ended(flow)?;
        self.finish_comb(st, wait)
    }

    /// A process without a sensitivity list that waits for a time, forever, more than once or
    /// in a called procedure is behavioural.
    fn waits_behavioural(&mut self, body: &[StmtId]) -> R<()> {
        let db = self.db;
        let mut all = Vec::new();
        deep(db, body, &mut all);
        let mut waits = 0;
        for &s in &all {
            match &db.stmts[s].kind {
                StmtKind::Wait {
                    on, until, timeout, ..
                } => {
                    waits += 1;
                    if timeout.is_some() {
                        return Err(Stop::Behav("it waits for a time".into()));
                    }
                    if on.is_empty() && until.is_none() {
                        return Err(Stop::Behav("it waits forever (`wait;`)".into()));
                    }
                }
                StmtKind::ProcCall(c) if self.callee(c.callee).waits => {
                    return Err(Stop::Behav(format!(
                        "it calls `{}`, which can wait",
                        db.decls[c.callee].name
                    )));
                }
                _ => {}
            }
        }
        if waits != 1 {
            return Err(Stop::Behav(format!("it has {waits} wait statements")));
        }
        Ok(())
    }

    /// `wait until <edge>;` first, then the body.
    fn wait_form(&mut self, kept: &[StmtId]) -> R<Fit> {
        let db = self.db;
        let mut all = Vec::new();
        deep(db, kept, &mut all);
        let Some((i, w)) = kept
            .iter()
            .enumerate()
            .find(|(_, s)| matches!(db.stmts[**s].kind, StmtKind::Wait { .. }))
        else {
            let w = (all.iter())
                .find(|s| matches!(db.stmts[**s].kind, StmtKind::Wait { .. }))
                .expect("one wait counted");
            return out(
                codes::SHAPE,
                db.stmts[*w].span,
                "a clocked process of another shape: its `wait` is inside another statement",
            );
        };
        let StmtKind::Wait { on, until, .. } = &db.stmts[*w].kind else {
            unreachable!("found as a wait");
        };
        let Some((edge, extras)) = until.and_then(|u| self.edge_head(u)) else {
            return out(
                codes::WAIT,
                db.stmts[*w].span,
                "a wait that is not `wait until <edge>;`",
            );
        };
        if i != 0 {
            return out(
                codes::SHAPE,
                db.stmts[kept[0]].span,
                "a clocked process of another shape: statements before its `wait until <edge>;`",
            );
        }
        let (leaf, el) = self.clock_elem(edge.sig)?;
        // The wait must resume on every event of the clock; events of other signals leave
        // the edge test false.
        if !on.is_empty() {
            let mut s = ESet::default();
            self.lsps_into(on, &mut s);
            if s.first_missing(u64::from(leaf.0), el, el + 1).is_some() {
                return out(
                    codes::SHAPE,
                    db.stmts[*w].span,
                    "a clocked process of another shape: its `wait` does not name the clock in its `on` list",
                );
            }
        }
        self.clock = Some((leaf, el));
        let st = St::default();
        for x in extras {
            self.expr(x, &st)?;
        }
        self.stmts(&kept[1..], st)?;
        Ok(Fit {
            form: Form::Clocked,
            clock: Some((leaf, el, edge.rising)),
            reset: Vec::new(),
            outs: Vec::new(),
            dropped: self.dropped,
            reads: ESet::default(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn clocked(
        &mut self,
        wait: &ESet,
        reset: Option<(ExprId, &[StmtId])>,
        inner: &[StmtId],
        edge: yavs_sem::edge::EdgeShape,
        extras: Vec<ExprId>,
        span: Span,
    ) -> R<Fit> {
        let db = self.db;
        let (leaf, el) = self.clock_elem(edge.sig)?;
        if wait.first_missing(u64::from(leaf.0), el, el + 1).is_some() {
            return out(
                codes::SHAPE,
                span,
                format!(
                    "a clocked process of another shape: the sensitivity list does not name the clock {}",
                    self.e.leaf_name(leaf)
                ),
            );
        }
        self.clock = Some((leaf, el));
        let mut fit = Fit {
            form: Form::Clocked,
            clock: Some((leaf, el, edge.rising)),
            reset: Vec::new(),
            outs: Vec::new(),
            dropped: 0,
            reads: ESet::default(),
        };
        if let Some((r, a)) = reset {
            // The condition of an asynchronous reset: signals of its sensitivity list, not the
            // clock (`rst = '1'`, `a = '1' or b = '1'`). An edge test there is no trigger.
            let clock = self.clock.take();
            let mut before = std::mem::take(&mut self.reads);
            let mut st = St::default();
            let rd = self.expr(r, &st)?;
            let rs = std::mem::take(&mut self.reads);
            if rs.is_empty() {
                return out(
                    codes::SHAPE,
                    db.exprs[r].span,
                    "a clocked process of another shape: the branch before the edge test tests no signal, so it is not an asynchronous reset",
                );
            }
            if rs.first_missing(u64::from(leaf.0), el, el + 1).is_none() {
                return out(
                    codes::SHAPE,
                    db.exprs[r].span,
                    "a clocked process of another shape: the asynchronous reset condition reads the clock",
                );
            }
            if let Some(l) = self.missing(&rs, wait) {
                return out(
                    codes::SHAPE,
                    db.exprs[r].span,
                    format!(
                        "a clocked process of another shape: the asynchronous reset reads {l}, which the sensitivity list does not name"
                    ),
                );
            }
            fit.reset = rs.iter().map(|(k, _, _)| LeafId(k as u32)).collect();
            before.union(&rs);
            fit.reset.dedup();
            // While the reset holds, the branch is combinational logic of what it reads.
            self.comb = true;
            st.ctl = Rc::new(self.flat(&rd));
            let flow = self.stmts(a, st)?;
            let st = self.ended(flow)?;
            self.assigns_what_it_drives()?;
            self.stale_reads()?;
            let reads = std::mem::replace(&mut self.reads, before);
            self.reads.union(&reads);
            if let Some(l) = self.missing(&reads, wait) {
                return out(
                    codes::SENSITIVITY,
                    db.exprs[r].span,
                    format!(
                        "an incomplete sensitivity list: the reset branch reads {l}, which the list does not name"
                    ),
                );
            }
            for (&k, segs) in &st.vals {
                if k & VAR == 0 {
                    for (lo, hi, d) in segs {
                        fit.outs.push(Out {
                            leaf: k,
                            lo: *lo,
                            hi: *hi,
                            dep: d.clone(),
                        });
                    }
                }
            }
            self.comb = false;
            self.clock = clock;
        }
        let st = St::default();
        for x in extras {
            self.expr(x, &st)?;
        }
        self.stmts(inner, st)?;
        fit.dropped = self.dropped;
        Ok(fit)
    }

    /// The fall-through state of a body that must end normally.
    fn ended(&mut self, flow: Flow) -> R<St> {
        if let Some((s, _)) = flow.exits.first().or(flow.nexts.first()) {
            // Analysis ties every `exit` and `next` to a loop around it.
            return bug(
                self.db.stmts[*s].span,
                "an `exit` or `next` of a loop outside this process or subprogram",
            );
        }
        Ok(flow.fall.unwrap_or_default())
    }

    /// Check a combinational process at its end: every driven element assigned on every path,
    /// every read in its wait set; its dependences.
    fn finish_comb(&mut self, st: St, wait: ESet) -> R<Fit> {
        self.assigns_what_it_drives()?;
        self.stale_reads()?;
        let mut outs = Vec::new();
        for (l, (lo, hi)) in self.driven() {
            // An element no path assigns keeps its initial value: a constant, no latch.
            let mut want = ESet::default();
            want.add(l, lo, hi);
            let want = want.intersect(&self.sig_any);
            if let Some((_, x, _)) = want.minus(&st.def).iter().next() {
                let mut text = format!(
                    "a latch: {} is not assigned on every path",
                    self.elem_name(l, x)
                );
                let mut at = self.span();
                if let Some((_, s)) = self
                    .lost
                    .iter()
                    .find(|(s, _)| s.first_missing(l, x, x + 1).is_none())
                {
                    text.push_str(" (this statement leaves it unassigned)");
                    at = *s;
                }
                return out(codes::LATCH, at, text);
            }
            for (s_lo, s_hi, d) in st.vals.get(&l).into_iter().flatten() {
                let (a, z) = ((*s_lo).max(lo), (*s_hi).min(hi));
                if a < z {
                    outs.push(Out {
                        leaf: l,
                        lo: a,
                        hi: z,
                        dep: shift(d, a - s_lo),
                    });
                }
            }
        }
        if let Some(l) = self.missing(&self.reads.clone(), &wait) {
            let (k, _, _) = self.reads.minus(&wait).iter().next().expect("one missing");
            return out(
                codes::SENSITIVITY,
                self.read_at.get(&k).copied().unwrap_or(self.span()),
                format!(
                    "an incomplete sensitivity list: the process reads {l}, which the list does not name"
                ),
            );
        }
        Ok(Fit {
            form: Form::Comb,
            clock: None,
            reset: Vec::new(),
            outs,
            dropped: self.dropped,
            reads: ESet::default(),
        })
    }

    /// A variable element read before it was assigned on some path, and assigned somewhere:
    /// the value of the previous run, state. One never assigned keeps its initial value.
    fn stale_reads(&self) -> R<()> {
        for (need, span, decl) in &self.undef_reads {
            if let Some((k, x, _)) = need.intersect(&self.var_any).iter().next() {
                return out(
                    codes::STATE,
                    *span,
                    format!(
                        "a variable read before it is assigned: {} is read here before it is assigned on every path",
                        self.var_name(*decl, k, x)
                    ),
                );
            }
        }
        Ok(())
    }

    /// Every signal element the analysis saw assigned is one elaboration made a driver for: a
    /// cross-check of the element mapping the latch rule relies on.
    fn assigns_what_it_drives(&self) -> R<()> {
        let mut driven = ESet::default();
        for (l, (a, z)) in self.driven() {
            driven.add(l, a, z);
        }
        if let Some((k, x, _)) = self.sig_any.minus(&driven).iter().next() {
            return bug(
                self.span(),
                format!(
                    "an assignment to {}, which elaboration does not list among the process's drivers",
                    self.elem_name(k, x)
                ),
            );
        }
        Ok(())
    }

    /// The first element of `reads` not in `wait`, named.
    fn missing(&self, reads: &ESet, wait: &ESet) -> Option<String> {
        let (k, a, _) = reads.minus(wait).iter().next()?;
        Some(self.elem_name(k, a))
    }

    /// The elements process `p` drives, from elaboration.
    fn driven(&self) -> Vec<(u64, (u32, u32))> {
        let nets = &self.e.nets;
        (self.p.drivers.iter())
            .map(|&d| {
                let d = &nets.drivers[d];
                (u64::from(d.leaf.0), (d.range.offset, d.range.end()))
            })
            .collect()
    }

    /// The signal elements a process waits on: its list, or for `all` what elaboration took;
    /// a name that does not resolve exactly adds nothing (under-approximated).
    fn wait_set(&mut self, sens: &Sensitivity) -> ESet {
        let db = self.db;
        let mut w = ESet::default();
        match sens {
            Sensitivity::List(l) => self.lsps_into(l, &mut w),
            Sensitivity::All => {
                let info = match &db.stmts[self.p.stmt].kind {
                    StmtKind::Process { info, .. } => info.reads_all.clone(),
                    _ => None,
                };
                if let Some(l) = info {
                    self.lsps_into(&l, &mut w);
                }
            }
            Sensitivity::None => {}
        }
        w
    }

    /// The elements of longest static prefixes, those that resolve exactly.
    fn lsps_into(&mut self, lsps: &[yavs_sem::hir::Lsp], out: &mut ESet) {
        for l in lsps {
            let yavs_sem::hir::ObjRoot::Decl(d) = l.root else {
                continue;
            };
            let Some(o) = self.e.signal(self.p.scope, d) else {
                continue;
            };
            let mut sels = Vec::new();
            let mut ok = true;
            for s in l.path.iter() {
                match s {
                    yavs_sem::hir::LspStep::Field(f) => sels.push(Sel::Field(*f)),
                    yavs_sem::hir::LspStep::Index(ix) => {
                        let v: Option<Vec<i64>> = ix
                            .iter()
                            .map(|&x| self.fold(x).and_then(|c| c.as_int()))
                            .collect();
                        match v {
                            Some(v) => sels.push(Sel::Index(v)),
                            None => ok = false,
                        }
                    }
                    yavs_sem::hir::LspStep::Slice(r) => match self.range_bounds(r) {
                        Some(b) => sels.push(Sel::Slice(b)),
                        None => ok = false,
                    },
                    yavs_sem::hir::LspStep::Rebase(_) => ok = false,
                }
                if !ok {
                    break;
                }
            }
            if !ok {
                continue;
            }
            if let (parts, true) = self.sig_parts(o, &sels) {
                for (k, a, z) in parts {
                    out.add(k, a, z);
                }
            }
        }
    }

    // -- names ------------------------------------------------------------------------------

    /// The elements of signal object `o` selected by `sels`, and whether exactly.
    fn sig_parts(&mut self, o: ObjId, sels: &[Sel]) -> (Parts, bool) {
        let obj = self.e.nets.obj(o);
        // A port collapsed into its actual has the actual's leaves, whose record paths are the
        // actual's: select in its own layout, by position (`collapse.rs`).
        let collapsed = obj.leaves.0 != obj.leaves.1 && self.e.nets.leaf(obj.leaves.0).obj != o;
        if obj.parts.is_empty() && !collapsed {
            return match self.e.nets.select(o, sels) {
                Ok((ps, _)) => (
                    ps.iter()
                        .map(|(l, r)| (u64::from(l.0), r.offset, r.end()))
                        .collect(),
                    true,
                ),
                Err(_) => (self.whole(o), false),
            };
        }
        // Select in a layout of its own shape, then move to the actual's leaves and parts.
        let local = match self.part_obj.get(&o) {
            Some(&l) => l,
            None => {
                let l = add_layout(&mut self.vars, obj.decl, obj.name, obj.scope, &obj.shape);
                self.part_obj.insert(o, l);
                l
            }
        };
        let first = self.vars.obj(local).leaves.0.0;
        match self.vars.select(local, sels) {
            Ok((ps, _)) => (
                ps.iter()
                    .map(|(l, r)| {
                        let k = (l.0 - first) as usize;
                        let base = obj.part(k).map_or(0, |p| p.offset);
                        (
                            u64::from(obj.leaves.0.0 + k as u32),
                            base + r.offset,
                            base + r.end(),
                        )
                    })
                    .collect(),
                true,
            ),
            Err(_) => (self.whole(o), false),
        }
    }

    fn whole(&self, o: ObjId) -> Vec<(u64, u32, u32)> {
        let nets = &self.e.nets;
        let obj = nets.obj(o);
        (obj.leaves.0.0..obj.leaves.1.0)
            .enumerate()
            .map(|(k, l)| {
                let r = obj
                    .part(k)
                    .unwrap_or(yavs_kernel::Range::new(0, nets.leaf(LeafId(l)).ty.len));
                (u64::from(l), r.offset, r.end())
            })
            .collect()
    }

    /// The layout object of a process variable (`None`: one element standing for all of it).
    fn var_layout(&mut self, d: DeclId) -> Option<ObjId> {
        if let Some(o) = self.var_obj.get(&d) {
            return *o;
        }
        let e = self.e;
        let key = (Some(self.p.scope), d);
        let up = (e.hier.scope(self.p.scope).parent, d);
        let yavs_sem::decl::Designator::Ident(name) = self.db.decls[d].name else {
            self.var_obj.insert(d, None);
            return None;
        };
        let o = (e.var_shapes.get(&key).or_else(|| e.var_shapes.get(&up)))
            .map(|sh| add_layout(&mut self.vars, d, name, self.p.scope, sh));
        self.var_obj.insert(d, o);
        o
    }

    fn var_parts(&mut self, d: DeclId, sels: &[Sel]) -> (Vec<(u64, u32, u32)>, bool) {
        let Some(o) = self.var_layout(d) else {
            // One coarse element per variable, keyed by its declaration.
            return (
                vec![(VAR | (1 << 40) | d.index() as u64, 0, 1)],
                sels.is_empty(),
            );
        };
        let whole = |v: &NetGraph| -> Vec<(u64, u32, u32)> {
            let ob = v.obj(o);
            (ob.leaves.0.0..ob.leaves.1.0)
                .map(|l| (VAR | u64::from(l), 0, v.leaf(LeafId(l)).ty.len))
                .collect()
        };
        match self.vars.select(o, sels) {
            Ok((ps, _)) => (
                ps.iter()
                    .map(|(l, r)| (VAR | u64::from(l.0), r.offset, r.end()))
                    .collect(),
                true,
            ),
            Err(_) => (whole(&self.vars), false),
        }
    }

    /// Resolve a name; `None` when `x` is not a name of an object.
    fn name(&mut self, x: ExprId, st: &St) -> R<Option<Name>> {
        let db = self.db;
        let ex = &db.exprs[x];
        Ok(Some(match &ex.kind {
            ExprKind::Obj(d) => match &db.decls[*d].kind {
                DeclKind::Alias(Alias::Object { target, subtype }) => {
                    let Some(mut n) = self.name(*target, st)? else {
                        return Ok(None);
                    };
                    // An alias with an array subtype indexes in its own index space.
                    if subtype.is_some_and(|st| {
                        matches!(db.types[db.base(st)].kind, TypeKind::Array { .. })
                    }) {
                        n.stopped = true;
                    }
                    n
                }
                DeclKind::Object(o) => {
                    let root = match (o.class, &o.role) {
                        (_, ObjRole::ImplicitSignal) => {
                            return out(
                                codes::HISTORY,
                                ex.span,
                                "an implicit signal (`'stable`, `'quiet`, `'delayed`, \
                                 `'transaction`)",
                            );
                        }
                        (_, ObjRole::Guard) => {
                            return out(
                                codes::GUARDED,
                                ex.span,
                                "the `GUARD` signal of a guarded block",
                            );
                        }
                        (ObjClass::Signal, _) => match self.e.signal(self.p.scope, *d) {
                            Some(o) => Root::Sig(o),
                            None => {
                                return out(
                                    codes::ANALYSIS,
                                    ex.span,
                                    format!(
                                        "the signal `{}`, which yavs cannot place in the design",
                                        db.decls[*d].name
                                    ),
                                );
                            }
                        },
                        (ObjClass::Variable, _) => Root::Var(*d),
                        // A memory held in a shared variable, in the clock-edge body of a
                        // clocked process: a state variable of it (A15), when no other
                        // process uses it (`region::shared_rule`).
                        (ObjClass::SharedVariable, _) if !self.comb && self.clock.is_some() => {
                            Root::Var(*d)
                        }
                        (ObjClass::SharedVariable, _) => {
                            return out(
                                codes::TESTBENCH,
                                ex.span,
                                format!("the shared variable `{}`", db.decls[*d].name),
                            );
                        }
                        (ObjClass::File, _) => {
                            return Err(Stop::Behav(format!(
                                "it uses the file `{}`",
                                db.decls[*d].name
                            )));
                        }
                        (ObjClass::Constant, _) => {
                            self.no_z_const(x)?;
                            Root::Const
                        }
                    };
                    Name {
                        root,
                        decl: Some(*d),
                        sels: Vec::new(),
                        stopped: false,
                        idx: ESet::default(),
                    }
                }
                _ => return Ok(None),
            },
            ExprKind::External(_) => {
                return out(codes::EXTERNAL, ex.span, "an external name");
            }
            ExprKind::Field { prefix, elem } => {
                let Some(mut n) = self.name(*prefix, st)? else {
                    return Ok(None);
                };
                if !n.stopped {
                    n.sels.push(Sel::Field(*elem));
                }
                n
            }
            ExprKind::Index { prefix, indices } => {
                let Some(mut n) = self.name(*prefix, st)? else {
                    return Ok(None);
                };
                let mut vals = Vec::new();
                for &i in indices.iter() {
                    let d = self.expr(i, st)?;
                    let d = self.flat(&d);
                    n.idx.union(&d);
                    vals.push(self.fold(i).and_then(|c| c.as_int()));
                }
                match vals.into_iter().collect::<Option<Vec<i64>>>() {
                    Some(v) if !n.stopped => n.sels.push(Sel::Index(v)),
                    _ => n.stopped = true,
                }
                n
            }
            ExprKind::Slice { prefix, range } => {
                let Some(mut n) = self.name(*prefix, st)? else {
                    return Ok(None);
                };
                if let RangeC::Explicit { left, right, .. } = range {
                    for x in [*left, *right] {
                        let d = self.expr(x, st)?;
                        let d = self.flat(&d);
                        n.idx.union(&d);
                    }
                }
                match self.range_bounds(range) {
                    Some(b) if !n.stopped => n.sels.push(Sel::Slice(b)),
                    _ => n.stopped = true,
                }
                n
            }
            ExprKind::Deref { .. } => {
                return out(
                    codes::TESTBENCH,
                    ex.span,
                    "an access value (dynamic memory)",
                );
            }
            _ => return Ok(None),
        }))
    }

    fn place(&mut self, x: ExprId, st: &St) -> R<Option<Place>> {
        let Some(n) = self.name(x, st)? else {
            return Ok(None);
        };
        Ok(self.place_of(n))
    }

    fn place_of(&mut self, n: Name) -> Option<Place> {
        let decl = n.decl;
        match n.root {
            Root::Const => None,
            Root::Sig(o) => {
                let (parts, exact) = self.sig_parts(o, &n.sels);
                Some(Place {
                    parts,
                    precise: exact && !n.stopped,
                    sig: true,
                    idx: n.idx,
                    decl,
                })
            }
            Root::Var(d) => {
                let (parts, exact) = self.var_parts(d, &n.sels);
                Some(Place {
                    parts,
                    precise: exact && !n.stopped,
                    sig: false,
                    idx: n.idx,
                    decl,
                })
            }
        }
    }

    /// The one static element a clock or reset name denotes.
    fn clock_elem(&mut self, x: ExprId) -> R<(LeafId, u32)> {
        let st = St::default();
        let span = self.db.exprs[x].span;
        match self.place(x, &st)? {
            Some(Place {
                parts,
                precise: true,
                sig: true,
                ..
            }) if parts.len() == 1 && parts[0].2 == parts[0].1 + 1 => {
                Ok((LeafId(parts[0].0 as u32), parts[0].1))
            }
            _ => out(
                codes::SHAPE,
                span,
                "a clocked process of another shape: a clock or reset that is not one fixed element of a signal",
            ),
        }
    }

    // -- static values ----------------------------------------------------------------------

    /// The value of `x` when elaboration knows it: literals, generics, constants, loop
    /// parameters being enumerated, array attributes of objects with known bounds, and
    /// predefined operations of those.
    /// `fold_raw` of a name or call with the index ranges of its subtype at every level
    /// (`bounded`).
    fn fold(&mut self, x: ExprId) -> Option<ConstValue> {
        let v = self.fold_raw(x)?;
        let db = self.db;
        if matches!(v, ConstValue::Array(_) | ConstValue::Record(_)) && named(&db.exprs[x].kind) {
            let (e, scope) = (self.e, self.p.scope);
            return bounded(db, v, db.exprs[x].subtype, &mut |r| {
                self.range_bounds(r).or_else(|| match r {
                    RangeC::Explicit { left, .. } => e.range(scope, *left),
                    RangeC::Attr { .. } => None,
                })
            });
        }
        Some(v)
    }

    /// `fold(x)`, shared through `consts` when `x` names a constant elaboration gave a value.
    fn fold_shared(&mut self, x: ExprId) -> Option<Rc<ConstValue>> {
        let db = self.db;
        if let ExprKind::Obj(d) = db.exprs[x].kind
            && matches!(&db.decls[d].kind, DeclKind::Object(o) if o.class == ObjClass::Constant)
            && db.value(x).is_none()
            && !self.frames.last().is_some_and(|f| f.contains_key(&d))
            && !self.opaque.contains(&d)
            && !self.loopv.contains_key(&d)
            && self.e.value(self.p.scope, d).is_some()
        {
            let key = (self.p.scope, d, db.exprs[x].subtype);
            if let Some(v) = self.consts.get(&key) {
                return v.clone();
            }
            let v = self.fold(x).map(Rc::new);
            self.consts.insert(key, v.clone());
            return v;
        }
        self.fold(x).map(Rc::new)
    }

    fn fold_raw(&mut self, x: ExprId) -> Option<ConstValue> {
        let (db, e) = (self.db, self.e);
        if let Some(v) = db.value(x) {
            return Some(v.clone());
        }
        // A globally static call of a pure function elaboration evaluated for this process
        // without a report (a `while` loop over a string generic): its scalar value, under the
        // lowering's rule (`lower::call_fits`): elaboration checks no argument's or result's
        // subtype, so each must fit, or take any value of its type (`takes_any`).
        if self.frames.is_empty()
            && let Some(rc) = e.region_calls.get(&(self.p.scope, x))
            && rc.reports.is_empty()
            && let Some(v @ (ConstValue::Int(_) | ConstValue::Enum(_))) = &rc.val
            && let ExprKind::Call { callee, args, .. } = &db.exprs[x].kind
            && let Some(sp) = db.decls[*callee].kind.subprogram()
            && sp.ret.is_some_and(|r| scalar_fits(db, r, v))
            && args.iter().all(|a| {
                let (&Actual::Expr(ax), Some(p)) =
                    (&a.actual, sp.params.get(usize::from(a.formal)))
                else {
                    return false;
                };
                takes_any(db, p.subtype)
                    || self
                        .fold(ax)
                        .is_some_and(|c| scalar_fits(db, p.subtype, &c))
            })
        {
            return Some(v.clone());
        }
        match &db.exprs[x].kind {
            ExprKind::Obj(d) => {
                if let Some(v) = self.frames.last().and_then(|f| f.get(d)) {
                    return Some(v.clone());
                }
                if self.opaque.contains(d) {
                    return None;
                }
                if let Some(v) = self.loopv.get(d) {
                    return Some(v.clone());
                }
                if self.frames.is_empty()
                    && let Some(v) = self.vknown.get(d)
                {
                    return Some(v.clone());
                }
                if let Some(v) = e.value(self.p.scope, *d) {
                    return Some(v.clone());
                }
                // A constant elaboration left to run time: its initial value, folded here.
                if let DeclKind::Object(o) = &db.decls[*d].kind
                    && o.class == ObjClass::Constant
                    && matches!(o.role, ObjRole::Plain | ObjRole::Deferred)
                    && let Some(init) = o.default
                    && self.frames.len() < 16
                {
                    self.frames.push(HashMap::new());
                    let v = self.fold(init).filter(scalar);
                    self.frames.pop();
                    return v;
                }
            }
            ExprKind::Attr {
                attr,
                prefix: pre @ (AttrPrefix::Expr(_) | AttrPrefix::Subtype(_)),
                args,
            } if matches!(
                attr,
                PredefAttr::Length
                    | PredefAttr::Left
                    | PredefAttr::Right
                    | PredefAttr::High
                    | PredefAttr::Low
            ) =>
            {
                let dim = match args.first() {
                    Some(&a) => u32::try_from(self.fold(a)?.as_int()? - 1).ok()?,
                    None => 0,
                };
                let b = match pre {
                    AttrPrefix::Expr(p) => self.prefix_bounds(*p, dim)?,
                    AttrPrefix::Subtype(st) => self.subtype_bounds(*st, dim)?,
                    AttrPrefix::Entity(_) => return None,
                };
                let (lo, hi) = b.low_high();
                return Some(ConstValue::Int(match attr {
                    PredefAttr::Length => b.len() as i64,
                    PredefAttr::Left => b.left,
                    PredefAttr::Right => b.right,
                    PredefAttr::High => hi,
                    _ => lo,
                }));
            }
            // `math_real`'s `log2`, `ceil` and `floor` of a constant (`ceil(log2(real(n)))`), where
            // the native computes them as `f64` does: a positive argument below `2**31`.
            ExprKind::Call { callee, args, .. }
                if let [a] = &args[..]
                    && let Actual::Expr(ax) = a.actual
                    && let Some(m) = (db.decls[*callee].kind.subprogram())
                        .and_then(|sp| crate::lower::ieee_op(sp.mangled.as_str())) =>
            {
                use yavs_ir::ieee::{Math, Op};
                let Some(ConstValue::Real(r)) = self.fold(ax) else {
                    return None;
                };
                if !(r > 0.0 && r < 2147483648.0) {
                    return None;
                }
                return Some(ConstValue::Real(match m {
                    Op::Math(Math::Log2) => r.log2(),
                    Op::Math(Math::Ceil) => r.ceil(),
                    Op::Math(Math::Floor) => r.floor(),
                    _ => return None,
                }));
            }
            // A pure function of constants (`is_write_mode(regs(i).mode)`, `log2ceil(n)`).
            ExprKind::Call {
                callee,
                args,
                method_of: None,
                ..
            } if matches!(
                db.decls[*callee]
                    .kind
                    .subprogram()
                    .map(|s| (&s.kind, &s.body)),
                Some((SubprogramKind::Function { pure: true }, Body::Hir { .. }))
            ) =>
            {
                return self.call_value(*callee, args);
            }
            // An element or field of a constant (`table(i)`, `grid(r, c)`, `cfg.width`).
            ExprKind::Index { prefix, indices } => {
                if let Some(p) = self.fold_shared(*prefix)
                    && let ConstValue::Array(a) = &*p
                {
                    let ix: Option<Vec<i64>> = (indices.iter())
                        .map(|&i| self.fold(i).and_then(|c| c.as_int()))
                        .collect();
                    return const_elem(a, &ix?);
                }
            }
            ExprKind::Field { prefix, elem } => {
                if let Some(p) = self.fold_shared(*prefix)
                    && let ConstValue::Record(fs) = &*p
                {
                    return fs.get(*elem as usize).cloned();
                }
            }
            _ => {}
        }
        if let Some(v) = record_fold(db, x, &mut |y| self.fold(y)) {
            return Some(v);
        }
        db.fold_with(x, &mut |y| self.fold(y)).ok().flatten()
    }

    /// Run pure function `f` on constant arguments: scalar variables, `if`, `case`, loops,
    /// `return`; `None` for anything else, or past a step or depth limit.
    fn call_value(&mut self, f: DeclId, args: &[yavs_sem::hir::Arg]) -> Option<ConstValue> {
        let db = self.db;
        let sp = db.decls[f].kind.subprogram()?;
        let Body::Hir { scope, stmts } = &sp.body else {
            return None;
        };
        if self.frames.len() >= 16 {
            return None;
        }
        // The body names its own declarations of the parameters (a package body repeats them).
        let own: Vec<DeclId> = (db.scopes[*scope].order.iter().copied())
            .filter(
                |&d| matches!(&db.decls[d].kind, DeclKind::Object(o) if o.role == ObjRole::Param),
            )
            .collect();
        let mut frame: HashMap<DeclId, ConstValue> = HashMap::new();
        for a in args {
            let i = usize::from(a.formal);
            let p = sp.params.get(i)?;
            let v = match &a.actual {
                Actual::Expr(x) => self.fold(*x)?,
                Actual::Default => self.fold(p.default?)?,
                _ => return None,
            };
            if !scalar(&v) && !scalar_record(&v) {
                return None;
            }
            let d = if own.len() == sp.params.len() {
                own[i]
            } else {
                p.decl?
            };
            frame.insert(d, v);
        }
        self.frames.push(frame);
        // Declarations of the body with initial values, in order. A constant that does not fold
        // (a message string) is unknown: a read of it gives up (`opaque`).
        let ok = true;
        let mut hidden = Vec::new();
        for &d in &db.scopes[*scope].order {
            if let DeclKind::Object(o) = &db.decls[d].kind
                && matches!(o.class, ObjClass::Constant | ObjClass::Variable)
                && o.role == ObjRole::Plain
            {
                match (o.default.and_then(|x| self.fold(x)))
                    .filter(|v| scalar(v) || scalar_record(v))
                {
                    Some(v) => {
                        self.frames.last_mut().expect("pushed").insert(d, v);
                    }
                    None if o.class == ObjClass::Constant && self.opaque.insert(d) => {
                        hidden.push(d);
                    }
                    None => {}
                }
            }
        }
        let mut steps = 10_000usize;
        // The asserts of the `ieee` and `std` bodies are library warnings, which members do
        // not give (A6): `quiet_only` is for the design's.
        let quiet = self.quiet_only;
        self.quiet_only &= !["ieee.", "std."]
            .iter()
            .any(|p| sp.mangled.as_str().starts_with(p));
        let r = if ok {
            match self.run_pure(stmts, &mut steps) {
                Pure::Return(v) => Some(v),
                _ => None,
            }
        } else {
            None
        };
        self.quiet_only = quiet;
        for d in hidden {
            self.opaque.remove(&d);
        }
        self.frames.pop();
        r
    }

    /// Run a statement list of a pure function on constants. Every value it cannot fold gives
    /// up the whole call (`Pure::GiveUp`), never a fall-through.
    fn run_pure(&mut self, body: &[StmtId], steps: &mut usize) -> Pure {
        // `Some(v)` or give the call up.
        macro_rules! or_give_up {
            ($e:expr) => {
                match $e {
                    Some(v) => v,
                    None => return Pure::GiveUp,
                }
            };
        }
        let db = self.db;
        for &s in body {
            *steps = or_give_up!(steps.checked_sub(1));
            let flow = match &db.stmts[s].kind {
                // An assert shown to hold says nothing; any other, or a report, would.
                StmtKind::Assert { cond, .. } if self.quiet_only => {
                    if self.static_bool(*cond) != Some(true) {
                        return Pure::GiveUp;
                    }
                    Pure::Fall
                }
                StmtKind::Report { .. } if self.quiet_only => return Pure::GiveUp,
                StmtKind::Null | StmtKind::Assert { .. } | StmtKind::Report { .. } => Pure::Fall,
                StmtKind::VarAssign {
                    target,
                    rhs: Rhs::Simple(v),
                } => {
                    let ExprKind::Obj(d) = db.exprs[target.expr].kind else {
                        return Pure::GiveUp;
                    };
                    // Only a scalar variable of the function itself.
                    let local = matches!(&db.decls[d].kind,
                        DeclKind::Object(o) if o.class == ObjClass::Variable && o.role == ObjRole::Plain);
                    let v = or_give_up!(self.fold(*v).filter(scalar));
                    if !local {
                        return Pure::GiveUp;
                    }
                    or_give_up!(self.frames.last_mut()).insert(d, v);
                    Pure::Fall
                }
                StmtKind::Return(Some(x)) => Pure::Return(or_give_up!(
                    self.fold(*x).filter(|v| scalar(v) || scalar_record(v))
                )),
                StmtKind::If { branches, else_ } => {
                    let mut taken = else_.as_deref();
                    for (c, b) in branches.iter() {
                        if or_give_up!(self.static_bool(*c)) {
                            taken = Some(b);
                            break;
                        }
                    }
                    match taken {
                        Some(b) => self.run_pure(b, steps),
                        None => Pure::Fall,
                    }
                }
                StmtKind::Case { expr, alts, .. } => {
                    let v = or_give_up!(self.fold(*expr).and_then(|c| c.as_int()));
                    let mut taken = None;
                    for (cs, b) in alts.iter() {
                        let hit = match &cs.intervals {
                            Some(iv) => iv.iter().any(|&(a, z)| a <= v && v <= z),
                            None => (cs.choices.iter())
                                .any(|c| matches!(c, yavs_sem::hir::Choice::Others)),
                        };
                        if hit {
                            taken = Some(b);
                            break;
                        }
                    }
                    self.run_pure(or_give_up!(taken), steps)
                }
                StmtKind::Loop { scheme, body, .. } => {
                    let mut flow = Pure::Fall;
                    let mut i = 0u64;
                    loop {
                        *steps = or_give_up!(steps.checked_sub(1));
                        match scheme {
                            LoopScheme::For { param, range } => {
                                let b = or_give_up!(self.range_bounds(range));
                                if i >= b.len() {
                                    break;
                                }
                                let v = match b.dir {
                                    Direction::To => b.left + i as i64,
                                    Direction::Downto => b.left - i as i64,
                                };
                                let v = match (self.enum_param(*param), u32::try_from(v)) {
                                    (true, Ok(p)) => ConstValue::Enum(p),
                                    _ => ConstValue::Int(v),
                                };
                                or_give_up!(self.frames.last_mut()).insert(*param, v);
                            }
                            LoopScheme::While(c) => {
                                if !or_give_up!(self.static_bool(*c)) {
                                    break;
                                }
                            }
                            LoopScheme::Forever => {}
                        }
                        i += 1;
                        match self.run_pure(body, steps) {
                            Pure::Exit(l) if l == s => break,
                            Pure::Next(l) if l == s => {}
                            Pure::Fall => {}
                            other => {
                                flow = other;
                                break;
                            }
                        }
                    }
                    flow
                }
                StmtKind::Exit { loop_, when } | StmtKind::Next { loop_, when } => {
                    let go = match when {
                        Some(c) => or_give_up!(self.static_bool(*c)),
                        None => true,
                    };
                    match (go, matches!(db.stmts[s].kind, StmtKind::Exit { .. })) {
                        (false, _) => Pure::Fall,
                        (true, true) => Pure::Exit(*loop_),
                        (true, false) => Pure::Next(*loop_),
                    }
                }
                _ => Pure::GiveUp,
            };
            if !matches!(flow, Pure::Fall) {
                return flow;
            }
        }
        Pure::Fall
    }

    /// A loop parameter of an enumeration type: its values are positions.
    fn enum_param(&self, d: DeclId) -> bool {
        let db = self.db;
        matches!(&db.decls[d].kind,
            DeclKind::Object(o) if matches!(db.types[db.base(o.subtype)].kind, TypeKind::Enum { .. }))
    }

    /// A boolean condition elaboration knows.
    fn static_bool(&mut self, c: ExprId) -> Option<bool> {
        let db = self.db;
        if Some(db.base(db.exprs[c].subtype)) != db.well_known.boolean {
            return None;
        }
        match self.fold(c)? {
            ConstValue::Enum(p) => Some(p == 1),
            _ => None,
        }
    }

    /// The bounds of dimension `dim` of the array object or value `p`.
    fn prefix_bounds(&mut self, p: ExprId, dim: u32) -> Option<yavs_elab::Bounds> {
        if let Some(b) = self.subtype_bounds(self.db.exprs[p].subtype, dim) {
            return Some(b);
        }
        // A constant or generic: the bounds of its value.
        if let Some(ConstValue::Array(a)) = self.fold(p) {
            let &(left, dir, right) = a.bounds.get(dim as usize)?;
            return Some(yavs_elab::Bounds { left, dir, right });
        }
        let st = St::default();
        let saved = std::mem::take(&mut self.reads);
        let n = self.name(p, &st).ok().flatten();
        self.reads = saved;
        let n = n.filter(|n| !n.stopped)?;
        let shape = match n.root {
            Root::Sig(o) => self.e.nets.select(o, &n.sels).ok()?.1,
            Root::Var(d) => {
                let o = self.var_layout(d)?;
                self.vars.select(o, &n.sels).ok()?.1
            }
            Root::Const => return None,
        };
        match shape {
            Shape::Array { dims, .. } => dims.get(dim as usize).copied(),
            _ => None,
        }
    }

    fn range_bounds(&mut self, r: &RangeC) -> Option<yavs_elab::Bounds> {
        match r {
            RangeC::Explicit { left, dir, right } => Some(yavs_elab::Bounds {
                left: self.fold(*left)?.as_int()?,
                dir: *dir,
                right: self.fold(*right)?.as_int()?,
            }),
            RangeC::Attr {
                prefix,
                reverse,
                dim,
            } => {
                // `dim` counts from 1.
                let d = dim.saturating_sub(1);
                let b = match prefix {
                    AttrPrefix::Subtype(st) => self.subtype_bounds(*st, d)?,
                    AttrPrefix::Expr(x) => self.prefix_bounds(*x, d)?,
                    AttrPrefix::Entity(_) => return None,
                };
                Some(if *reverse {
                    yavs_elab::Bounds {
                        left: b.right,
                        dir: match b.dir {
                            Direction::To => Direction::Downto,
                            Direction::Downto => Direction::To,
                        },
                        right: b.left,
                    }
                } else {
                    b
                })
            }
        }
    }

    fn subtype_bounds(&mut self, st: SubtypeId, dim: u32) -> Option<yavs_elab::Bounds> {
        let db = self.db;
        match &db.subtypes[st].constraint {
            Constraint::Array { dims: Some(d), .. } => self.range_bounds(d.get(dim as usize)?),
            // Bounds elaboration evaluated (a function of generics, say) come from it.
            Constraint::Range(r) if dim == 0 => self.range_bounds(r).or_else(|| match r {
                RangeC::Explicit { left, .. } => self.e.range(self.p.scope, *left),
                RangeC::Attr { .. } => None,
            }),
            Constraint::None if dim == 0 => match &db.types[db.base(st)].kind {
                TypeKind::Enum { lits } => Some(yavs_elab::Bounds {
                    left: 0,
                    dir: Direction::To,
                    right: lits.len() as i64 - 1,
                }),
                _ => None,
            },
            _ => None,
        }
    }

    /// A folded value of a logic type holding `'Z'`.
    fn no_z(&self, x: ExprId, v: &ConstValue) -> R<()> {
        let db = self.db;
        let t = db.base(db.exprs[x].subtype);
        let elem = match &db.types[t].kind {
            TypeKind::Array { elem, .. } => db.base(*elem),
            _ => t,
        };
        let wk = &db.well_known;
        if Some(elem) == wk.bit
            || Some(elem) == wk.boolean
            || yavs_elab::elem_of(db, elem) != Elem::Logic
            || !matches!(db.types[elem].kind, TypeKind::Enum { .. })
        {
            return Ok(());
        }
        let z = match v {
            ConstValue::Enum(4) => true,
            ConstValue::Array(a) => match &a.elems {
                Elems::Bytes(b) => b.contains(&4),
                Elems::Runs(r) => r.iter().any(|(_, v)| *v == ConstValue::Enum(4)),
            },
            _ => false,
        };
        if z {
            return out(codes::HIGH_Z, db.exprs[x].span, "the value `'Z'`");
        }
        Ok(())
    }

    fn no_z_const(&mut self, x: ExprId) -> R<()> {
        if let Some(v) = self.fold(x) {
            self.no_z(x, &v)?;
        }
        Ok(())
    }

    // -- expressions --------------------------------------------------------------------------

    fn flat(&self, v: &Dv) -> ESet {
        match v {
            Dv::Flat(s) => (**s).clone(),
            Dv::Runs(rs) => {
                let mut s = ESet::default();
                for (n, d) in rs {
                    s.union(&d.set);
                    if let Some((k, b)) = d.ident {
                        s.add(k, b, b + n);
                    }
                }
                s
            }
        }
    }

    fn empty() -> Dv {
        Dv::Flat(Rc::new(ESet::default()))
    }

    /// What expression `x` depends on.
    fn expr(&mut self, x: ExprId, st: &St) -> R<Dv> {
        // Folds below read the variables known at this point (a branch walked before may have
        // left others).
        if !Rc::ptr_eq(&self.vknown, &st.known) {
            self.vknown = st.known.clone();
        }
        let db = self.db;
        let ex = &db.exprs[x];
        if let Some(c) = ex.value {
            self.no_z(x, &db.consts[c])?;
            return Ok(Self::empty());
        }
        let ty = db.base(ex.subtype);
        if matches!(
            db.types[ty].kind,
            TypeKind::Floating | TypeKind::Physical { .. } | TypeKind::UniversalReal
        ) && self.fold(x).is_none()
        {
            return out(
                codes::NOT_TWO_STATE,
                ex.span,
                "a real or physical (time) value computed at run time",
            );
        }
        match &ex.kind {
            ExprKind::Obj(_)
            | ExprKind::Field { .. }
            | ExprKind::Index { .. }
            | ExprKind::Slice { .. }
            | ExprKind::External(_)
            | ExprKind::Deref { .. } => self.read(x, st),
            ExprKind::Attr { attr, prefix, args } => {
                use PredefAttr::*;
                if let Some((leaf, el)) = self.edge_of_clock(x)? {
                    // The edge itself: the trigger, true whenever the body runs.
                    let _ = (leaf, el);
                    return Ok(Self::empty());
                }
                if matches!(
                    attr,
                    Event
                        | Active
                        | LastEvent
                        | LastActive
                        | LastValue
                        | Driving
                        | DrivingValue
                        | Delayed
                        | Stable
                        | Quiet
                        | Transaction
                ) {
                    let name = match attr {
                        Event => "event",
                        Active => "active",
                        LastEvent => "last_event",
                        LastActive => "last_active",
                        LastValue => "last_value",
                        Driving => "driving",
                        DrivingValue => "driving_value",
                        Delayed => "delayed",
                        Stable => "stable",
                        Quiet => "quiet",
                        _ => "transaction",
                    };
                    return out(codes::HISTORY, ex.span, format!("the attribute `'{name}`"));
                }
                let mut s = ESet::default();
                if matches!(
                    attr,
                    Image | Value | Pos | Val | Succ | Pred | Leftof | Rightof
                ) && let AttrPrefix::Expr(p) = prefix
                    && self.fold(*p).is_none()
                {
                    // `x'image`-like reads of a value given as the prefix (2019 forms).
                    let d = self.expr(*p, st)?;
                    s.union(&self.flat(&d));
                }
                for &a in args.iter() {
                    let d = self.expr(a, st)?;
                    s.union(&self.flat(&d));
                }
                Ok(Dv::Flat(Rc::new(s)))
            }
            ExprKind::Call {
                callee,
                args,
                method_of,
                ..
            } => {
                if method_of.is_some() {
                    return out(codes::TESTBENCH, ex.span, "a method of a protected object");
                }
                if self.edge_of_clock(x)?.is_some() {
                    return Ok(Self::empty());
                }
                if db.edge_shape(x).is_some() {
                    return out(
                        codes::HISTORY,
                        ex.span,
                        "an edge test of a signal other than the process's clock",
                    );
                }
                let sp = db.decls[*callee].kind.subprogram();
                // `a & b` of two logic vectors is a copy element by element.
                if let Some(sp) = sp
                    && let Body::Implicit(op) = &sp.body
                    && op.family == Family::Cat
                    && let [a, b] = &args[..]
                    && let (Actual::Expr(a), Actual::Expr(b)) = (&a.actual, &b.actual)
                    && self.is_logic(ex.subtype)
                    && self.is_logic(db.exprs[*a].subtype)
                    && self.is_logic(db.exprs[*b].subtype)
                {
                    let (da, db_) = (self.expr(*a, st)?, self.expr(*b, st)?);
                    let mut flat = self.flat(&da);
                    flat.union(&self.flat(&db_));
                    // The right operand holds the low elements.
                    return Ok(match (self.runs(db_, *b), self.runs(da, *a)) {
                        (Some(mut lo), Some(hi)) => {
                            lo.extend(hi);
                            Dv::Runs(lo)
                        }
                        _ => Dv::Flat(Rc::new(flat)),
                    });
                }
                // A pure function of constants is a constant, whatever its body computes on
                // the way (`natural(ceil(log2(real(n))))`).
                if let Some(c) = self.fold(x) {
                    self.no_z(x, &c)?;
                    return Ok(Self::empty());
                }
                // A pure function of the design given constants reads nothing: what it computes
                // on the way (a real, a vector result this evaluator does not hold) is the
                // lowering's to fold, or to refuse.
                if let Some(sp) = sp
                    && sp.kind == (SubprogramKind::Function { pure: true })
                    && matches!(sp.body, Body::Hir { .. })
                    && args.iter().all(|a| match a.actual {
                        Actual::Expr(ax) => self.fold(ax).is_some(),
                        Actual::Default => true,
                        _ => false,
                    })
                {
                    return Ok(Self::empty());
                }
                let s = self.call_deps(*callee, args, st, ex.span)?;
                Ok(Dv::Flat(Rc::new(s)))
            }
            ExprKind::Aggregate { assocs, .. } => {
                let mut s = ESet::default();
                for a in assocs.iter() {
                    for c in a.choices.iter() {
                        if let yavs_sem::hir::Choice::Expr(c) = c {
                            let d = self.expr(*c, st)?;
                            s.union(&self.flat(&d));
                        }
                    }
                    let d = self.expr(a.value, st)?;
                    s.union(&self.flat(&d));
                }
                Ok(Dv::Flat(Rc::new(s)))
            }
            ExprKind::Qualified { operand, .. }
            | ExprKind::Conv { operand, .. }
            | ExprKind::ImplicitConv { operand } => self.expr(*operand, st),
            ExprKind::Conditional { alts } => {
                let mut s = ESet::default();
                for (v, c) in alts.iter() {
                    let d = self.expr(*v, st)?;
                    s.union(&self.flat(&d));
                    if let Some(c) = c {
                        let d = self.expr(*c, st)?;
                        s.union(&self.flat(&d));
                    }
                }
                Ok(Dv::Flat(Rc::new(s)))
            }
            // An access value lives in a variable or comes from a call of an impure function;
            // a process declaring one is behavioural, and a call or operator taking or giving
            // one is one of a subprogram that uses access values (behavioural too).
            ExprKind::New { .. } | ExprKind::Null => {
                bug(ex.span, "an access value in an expression")
            }
            ExprKind::PrivateVar { .. } => out(codes::TESTBENCH, ex.span, "a protected object"),
            ExprKind::Const(_) | ExprKind::AttrValue { .. } | ExprKind::Open | ExprKind::Error => {
                Ok(Self::empty())
            }
        }
    }

    /// The value as runs of its elements, `None` when it is not one per element.
    fn runs(&mut self, v: Dv, x: ExprId) -> Option<Vec<(u32, Dep)>> {
        match v {
            Dv::Runs(r) => Some(r),
            Dv::Flat(s) => {
                let n = self.len_of(x)?;
                Some(vec![(n, Dep::set(s))])
            }
        }
    }

    /// The number of elements of a one-dimensional logic value, when static.
    fn len_of(&mut self, x: ExprId) -> Option<u32> {
        let db = self.db;
        if let Some(c) = db.exprs[x].value
            && let ConstValue::Array(a) = &db.consts[c]
            && let [(l, d, r)] = &a.bounds[..]
        {
            let n = if *d == Direction::To {
                r - l + 1
            } else {
                l - r + 1
            };
            return u32::try_from(n.max(0)).ok();
        }
        if !matches!(
            db.types[db.base(db.exprs[x].subtype)].kind,
            TypeKind::Array { .. }
        ) {
            return Some(1);
        }
        let b = self.subtype_bounds(db.exprs[x].subtype, 0)?;
        u32::try_from(b.len()).ok()
    }

    fn is_logic(&self, st: SubtypeId) -> bool {
        let db = self.db;
        match &db.types[db.base(st)].kind {
            TypeKind::Array { indexes, elem } => {
                indexes.len() == 1 && yavs_elab::elem_of(db, db.base(*elem)) == Elem::Logic
            }
            _ => false,
        }
    }

    /// `x` is an edge test of the clock (the leaf of `rising_edge(clk)` or `clk'event and ...`).
    fn edge_of_clock(&mut self, x: ExprId) -> R<Option<(LeafId, u32)>> {
        let Some(clk) = self.clock else {
            return Ok(None);
        };
        let db = self.db;
        let sig = match db.edge_shape(x) {
            Some(s) => s.sig,
            None => match &db.exprs[x].kind {
                ExprKind::Attr {
                    attr: PredefAttr::Event,
                    prefix: AttrPrefix::Expr(p),
                    ..
                } => *p,
                _ => return Ok(None),
            },
        };
        let st = St::default();
        match self.place(sig, &st)? {
            Some(Place {
                parts,
                precise: true,
                sig: true,
                ..
            }) if parts == [(u64::from(clk.0.0), clk.1, clk.1 + 1)] => Ok(Some(clk)),
            _ => Ok(None),
        }
    }

    /// The edge test a condition starts with (`E`, `E and X and ...`), and the other operands.
    fn edge_head(&self, c: ExprId) -> Option<(yavs_sem::edge::EdgeShape, Vec<ExprId>)> {
        let db = self.db;
        if let Some(s) = db.edge_shape(c) {
            return Some((s, Vec::new()));
        }
        let ExprKind::Call { callee, args, .. } = &db.exprs[c].kind else {
            return None;
        };
        let sp = db.decls[*callee].kind.subprogram()?;
        let Body::Implicit(op) = &sp.body else {
            return None;
        };
        if op.family != Family::And {
            return None;
        }
        let [a, b] = &args[..] else { return None };
        let (Actual::Expr(a), Actual::Expr(b)) = (&a.actual, &b.actual) else {
            return None;
        };
        let (s, mut extras) = self.edge_head(*a)?;
        extras.push(*b);
        Some((s, extras))
    }

    /// The first edge test anywhere in a statement list.
    fn find_edge(&self, body: &[StmtId]) -> Option<ExprId> {
        let db = self.db;
        let mut stmts = Vec::new();
        deep(db, body, &mut stmts);
        let mut found = None;
        for s in stmts {
            db.stmt_exprs(s, &mut |x, _| {
                db.walk_expr(x, &mut |y| {
                    if found.is_none() && db.edge_shape(y).is_some() {
                        found = Some(y);
                    }
                })
            });
        }
        found
    }

    /// A name read for its value.
    fn read(&mut self, x: ExprId, st: &St) -> R<Dv> {
        let Some(n) = self.name(x, st)? else {
            return self.unnamed(x, st);
        };
        // A part of a constant: what its indices read (`table(i)`).
        if matches!(n.root, Root::Const) {
            return Ok(Dv::Flat(Rc::new(n.idx)));
        }
        let Some(p) = self.place_of(n) else {
            return Ok(Self::empty());
        };
        let idx = Rc::new(p.idx.clone());
        if p.sig {
            for &(k, a, z) in &p.parts {
                self.reads.add(k, a, z);
                self.read_at.entry(k).or_insert(self.db.exprs[x].span);
            }
            if !p.precise {
                let mut s = p.idx;
                for &(k, a, z) in &p.parts {
                    s.add(k, a, z);
                }
                return Ok(Dv::Flat(Rc::new(s)));
            }
            return Ok(Dv::Runs(
                (p.parts.iter())
                    .map(|&(k, a, z)| {
                        (
                            z - a,
                            Dep {
                                ident: Some((k, a)),
                                set: idx.clone(),
                            },
                        )
                    })
                    .collect(),
            ));
        }
        // A variable: in combinational code an element read before it is assigned is state
        // when the process assigns it anywhere (checked at the end, `stale_reads`).
        if self.comb {
            let mut need = ESet::default();
            for &(k, a, z) in &p.parts {
                need.add(k, a, z);
            }
            let need = need.minus(&st.def);
            if !need.is_empty() {
                self.undef_reads.push((need, self.db.exprs[x].span, p.decl));
            }
        }
        let mut runs = Vec::new();
        for &(k, a, z) in &p.parts {
            let mut at = a;
            for (s_lo, s_hi, d) in st.vals.get(&k).into_iter().flatten() {
                let (lo, hi) = ((*s_lo).max(at), (*s_hi).min(z));
                if lo >= hi {
                    continue;
                }
                if lo > at {
                    runs.push((lo - at, Dep::set(idx.clone())));
                }
                runs.push((hi - lo, with_set(&shift(d, lo - s_lo), &idx)));
                at = hi;
            }
            if at < z {
                runs.push((z - at, Dep::set(idx.clone())));
            }
        }
        if !p.precise {
            let mut s = self.flat(&Dv::Runs(runs));
            s.union(&p.idx);
            return Ok(Dv::Flat(Rc::new(s)));
        }
        Ok(Dv::Runs(runs))
    }

    /// A part of a value that is not an object (an element of a function's result): everything
    /// the value and the indices read.
    fn unnamed(&mut self, x: ExprId, st: &St) -> R<Dv> {
        let db = self.db;
        let mut xs: Vec<ExprId> = Vec::new();
        match &db.exprs[x].kind {
            ExprKind::Index { prefix, indices } => {
                xs.push(*prefix);
                xs.extend(indices.iter());
            }
            ExprKind::Slice { prefix, range } => {
                xs.push(*prefix);
                if let RangeC::Explicit { left, right, .. } = range {
                    xs.extend([*left, *right]);
                }
            }
            ExprKind::Field { prefix, .. } => xs.push(*prefix),
            _ => {}
        }
        let mut s = ESet::default();
        for x in xs {
            let d = self.expr(x, st)?;
            s.union(&self.flat(&d));
        }
        Ok(Dv::Flat(Rc::new(s)))
    }

    /// The dependences of a call: its arguments, and the signals the subprogram reads itself.
    fn call_deps(
        &mut self,
        callee: DeclId,
        args: &[yavs_sem::hir::Arg],
        st: &St,
        span: Span,
    ) -> R<ESet> {
        let db = self.db;
        let c = self.callee(callee);
        self.callee_ok(callee, &c, span)?;
        let mut s = ESet::default();
        for a in args {
            for x in actual_exprs(&a.actual) {
                let d = self.expr(x, st)?;
                s.union(&self.flat(&d));
            }
        }
        for &d in &c.sig_reads {
            let Some(o) = self.e.signal(self.p.scope, d) else {
                return out(
                    codes::CALL,
                    span,
                    format!(
                        "a call of `{}`, which reads the signal `{}` that yavs cannot place in the design",
                        db.decls[callee].name, db.decls[d].name
                    ),
                );
            };
            for (k, a, z) in self.whole(o) {
                self.reads.add(k, a, z);
                s.add(k, a, z);
            }
        }
        Ok(s)
    }

    fn callee_ok(&self, callee: DeclId, c: &Callee, span: Span) -> R<()> {
        let name = &self.db.decls[callee].name;
        if c.files {
            return Err(Stop::Behav(format!(
                "it calls `{name}`, which uses files or text input and output"
            )));
        }
        if c.control {
            return Err(Stop::Behav(format!(
                "it calls `{name}`, which controls the simulation"
            )));
        }
        if let Some(w) = &c.opaque {
            return out(codes::CALL, span, format!("a call of `{name}`: {w}"));
        }
        // A procedure of a clocked process changing the process's own state is register logic.
        if let Some(w) = &c.writes_outside
            && self.comb
        {
            return out(codes::CALL, span, format!("a call of `{name}`: it {w}"));
        }
        if let Some((code, w)) = &c.bad {
            return out(code, span, format!("a call of `{name}`: it {w}"));
        }
        Ok(())
    }

    // -- subprograms --------------------------------------------------------------------------

    fn callee(&mut self, d: DeclId) -> Callee {
        if let Some(c) = self.callees.get(&d) {
            return c.clone();
        }
        if let Some(&depth) = self.visiting.get(&d) {
            // Recursion: what is known so far (the walk of `d` adds the rest); every summary
            // made under `d` until then is partial.
            self.partial = self.partial.min(depth);
            return Callee::default();
        }
        let depth = self.visiting.len();
        self.visiting.insert(d, depth);
        let outer = std::mem::replace(&mut self.partial, usize::MAX);
        let c = self.callee_walk(d);
        self.visiting.remove(&d);
        // Kept for every later call (of any process) only when complete: no subprogram still
        // being walked above `d` was called under it.
        if self.partial >= depth {
            self.callees.insert(d, c.clone());
            self.partial = outer;
        } else {
            self.partial = self.partial.min(outer);
        }
        c
    }

    fn callee_walk(&mut self, d: DeclId) -> Callee {
        let db = self.db;
        let decl = &db.decls[d];
        let mut c = Callee::default();
        let target = match &decl.kind {
            DeclKind::Alias(Alias::NonObject { target }) => Some(*target),
            _ => None,
        };
        if let Some(t) = target {
            return self.callee(t);
        }
        let Some(sp) = decl.kind.subprogram() else {
            c.opaque = Some("it has no VHDL body yavs can see".into());
            return c;
        };
        let procedure = sp.kind == SubprogramKind::Procedure;
        let pure = matches!(sp.kind, SubprogramKind::Function { pure: true });
        c.must_out = vec![false; sp.params.len()];
        for p in sp.params.iter() {
            if p.class == ObjClass::File
                || matches!(
                    db.types[db.base(p.subtype)].kind,
                    TypeKind::Access { .. } | TypeKind::File { .. }
                )
            {
                c.files = true;
            }
        }
        match &sp.body {
            Body::Implicit(_) => return c,
            Body::Native(_) => {
                // A procedure of the simulator (`std.env`, file and text procedures).
                c.control = procedure && !c.files;
                return c;
            }
            Body::Foreign(_) => {
                c.opaque = Some("it is a foreign subprogram".into());
                return c;
            }
            Body::Hir { .. } => {}
            _ if pure => return c,
            _ => {
                c.opaque = Some("its body is not in the analysed libraries".into());
                return c;
            }
        }
        let Body::Hir { scope, stmts } = &sp.body else {
            unreachable!("matched above");
        };
        let inside = |s: yavs_sem::ScopeId| {
            let mut x = Some(s);
            while let Some(y) = x {
                if y == *scope {
                    return true;
                }
                x = db.scopes[y].parent;
            }
            false
        };
        let params: HashSet<DeclId> = sp.params.iter().filter_map(|p| p.decl).collect();
        let mine = |x: DeclId| params.contains(&x) || inside(db.decls[x].scope);
        // A body apart from its declaration declares the formals again, in the same order.
        let again: Vec<DeclId> = (db.scopes[*scope].order.iter().copied())
            .filter(
                |&x| matches!(&db.decls[x].kind, DeclKind::Object(o) if o.role == ObjRole::Param),
            )
            .collect();
        let formal = |x: DeclId| {
            (sp.params.iter().position(|p| p.decl == Some(x)))
                .or_else(|| again.iter().position(|&y| y == x))
        };
        // The formals a top-level statement before any `return` assigns as a whole.
        for &s in stmts.iter() {
            match &db.stmts[s].kind {
                StmtKind::VarAssign {
                    target,
                    rhs: Rhs::Simple(_),
                }
                | StmtKind::SignalAssign {
                    target,
                    rhs: Rhs::Simple(_),
                    ..
                } => {
                    if let ExprKind::Obj(x) = db.exprs[target.expr].kind
                        && let Some(i) = formal(x)
                        && i < c.must_out.len()
                    {
                        c.must_out[i] = true;
                    }
                }
                StmtKind::Return(_) => break,
                _ if db.stmt_children(s).is_empty() => {}
                _ => {}
            }
        }
        let mut all = Vec::new();
        deep(db, stmts, &mut all);
        let mut calls = Vec::new();
        let mut bad: Option<(&'static str, String)> = None;
        for &s in &all {
            match &db.stmts[s].kind {
                StmtKind::Wait { .. } => c.waits = true,
                StmtKind::Assert { .. } | StmtKind::Report { .. } => continue,
                StmtKind::ProcCall(call) => calls.push(call.callee),
                StmtKind::Force { .. } | StmtKind::Release { .. } => {
                    bad.get_or_insert((codes::FORCE, "forces a signal".into()));
                }
                StmtKind::SignalAssign { delay, rhs, .. }
                    if !matches!(delay, yavs_sem::hir::Delay::Default)
                        || waveforms(rhs).any(|w| match w {
                            Waveform::Elements(e) => {
                                e.len() != 1 || e.iter().any(|x| x.after.is_some())
                            }
                            Waveform::Unaffected => false,
                        }) =>
                {
                    bad.get_or_insert((codes::DELAY, "assigns a signal with a delay".into()));
                }
                StmtKind::VarAssign { target, .. } | StmtKind::SignalAssign { target, .. } => {
                    if let Some(yavs_sem::lsp::NameRoot::Obj(yavs_sem::hir::ObjRoot::Decl(x))) =
                        db.name_root(target.expr)
                        && !mine(x)
                        && c.writes_outside.is_none()
                    {
                        c.writes_outside = Some(format!(
                            "assigns `{}`, which it is not given as a parameter",
                            db.decls[x].name
                        ));
                    }
                }
                _ => {}
            }
            db.stmt_exprs(s, &mut |x, _| {
                db.walk_expr(x, &mut |y| {
                    let ex = &db.exprs[y];
                    if ex.value.is_none()
                        && matches!(
                            db.types[db.base(ex.subtype)].kind,
                            TypeKind::Floating
                                | TypeKind::Physical { .. }
                                | TypeKind::UniversalReal
                        )
                    {
                        bad.get_or_insert((
                            codes::NOT_TWO_STATE,
                            "computes a real or physical (time) value".into(),
                        ));
                    }
                    if db.edge_shape(y).is_some() {
                        bad.get_or_insert((codes::HISTORY, "tests a signal edge".into()));
                    }
                    match &ex.kind {
                        ExprKind::Attr { attr, .. }
                            if attr.reads_value() || *attr == PredefAttr::Event =>
                        {
                            bad.get_or_insert((
                                codes::HISTORY,
                                "reads the history of a signal".into(),
                            ));
                        }
                        ExprKind::Call {
                            method_of: Some(_), ..
                        }
                        | ExprKind::PrivateVar { .. } => {
                            bad.get_or_insert((codes::TESTBENCH, "uses a protected object".into()));
                        }
                        ExprKind::Call { callee, .. } => calls.push(*callee),
                        ExprKind::Obj(_) => {
                            let x = match db.name_root(y) {
                                Some(yavs_sem::lsp::NameRoot::Obj(
                                    yavs_sem::hir::ObjRoot::Decl(x),
                                )) => x,
                                _ => return,
                            };
                            let x = &x;
                            if let DeclKind::Object(o) = &db.decls[*x].kind
                                && o.role == ObjRole::ImplicitSignal
                            {
                                bad.get_or_insert((
                                    codes::HISTORY,
                                    "reads an implicit signal (`'stable` and the like)".into(),
                                ));
                            }
                            if let DeclKind::Object(o) = &db.decls[*x].kind
                                && !mine(*x)
                            {
                                match o.class {
                                    ObjClass::Signal => c.sig_reads.push(*x),
                                    ObjClass::Variable if c.writes_outside.is_none() => {
                                        c.writes_outside = Some(format!(
                                            "uses `{}`, a variable outside it",
                                            db.decls[*x].name
                                        ));
                                    }
                                    ObjClass::SharedVariable => {
                                        bad.get_or_insert((
                                            codes::TESTBENCH,
                                            format!(
                                                "uses the shared variable `{}`",
                                                db.decls[*x].name
                                            ),
                                        ));
                                    }
                                    ObjClass::File => c.files = true,
                                    _ => {}
                                }
                            }
                        }
                        _ => {}
                    }
                })
            });
        }
        c.bad = bad;
        // Declarations of the body: files and access values.
        for &x in &db.scopes[*scope].order {
            if let DeclKind::Object(o) = &db.decls[x].kind
                && (o.class == ObjClass::File
                    || matches!(db.types[db.base(o.subtype)].kind, TypeKind::Access { .. }))
            {
                c.files = true;
            }
        }
        calls.sort_unstable();
        calls.dedup();
        for k in calls {
            let o = self.callee(k);
            c.waits |= o.waits;
            c.files |= o.files;
            c.control |= o.control;
            if c.opaque.is_none() {
                c.opaque = o
                    .opaque
                    .map(|w| format!("through `{}`: {w}", db.decls[k].name));
            }
            if c.bad.is_none() {
                c.bad = o
                    .bad
                    .map(|(code, w)| (code, format!("calls `{}`, which {w}", db.decls[k].name)));
            }
            if c.writes_outside.is_none() {
                c.writes_outside = o
                    .writes_outside
                    .map(|w| format!("calls `{}`, which {w}", db.decls[k].name));
            }
            c.sig_reads.extend(o.sig_reads);
        }
        c.sig_reads.sort_unstable();
        c.sig_reads.dedup();
        c
    }

    // -- statements ---------------------------------------------------------------------------

    /// After an assignment to `target`: the value of a tracked variable when `rhs` folds (with
    /// the values known before the statement), else forgotten; any tracked variable a target of
    /// another form names is forgotten.
    fn track(&mut self, st: &mut St, target: ExprId, rhs: Option<ExprId>) {
        let db = self.db;
        if let ExprKind::Obj(d) = &db.exprs[target].kind {
            if !self.vtrack.contains(d) {
                return;
            }
            let v = rhs
                .and_then(|x| self.fold(x))
                .filter(|v| matches!(v, ConstValue::Int(_) | ConstValue::Enum(_)));
            let known = Rc::make_mut(&mut st.known);
            match v {
                Some(v) => known.insert(*d, v),
                None => known.remove(d),
            };
            return;
        }
        // The object an indexed, sliced or selected target writes; every name of an aggregate.
        let mut x = target;
        while let ExprKind::Index { prefix, .. }
        | ExprKind::Slice { prefix, .. }
        | ExprKind::Field { prefix, .. } = &db.exprs[x].kind
        {
            x = *prefix;
        }
        let mut names = Vec::new();
        db.walk_expr(x, &mut |y| {
            if let ExprKind::Obj(d) = &db.exprs[y].kind {
                names.push(*d);
            }
        });
        for d in names {
            if st.known.contains_key(&d) {
                Rc::make_mut(&mut st.known).remove(&d);
            }
        }
    }

    fn stmts(&mut self, body: &[StmtId], st: St) -> R<Flow> {
        let mut flow = Flow::fall(st);
        for &s in body {
            let Some(cur) = flow.fall.take() else {
                break;
            };
            let f = self.stmt(s, cur)?;
            flow.fall = f.fall;
            flow.exits.extend(f.exits);
            flow.nexts.extend(f.nexts);
        }
        Ok(flow)
    }

    fn stmt(&mut self, s: StmtId, mut st: St) -> R<Flow> {
        self.vknown = st.known.clone();
        if self.enumerating > 0 {
            if self.budget == 0 {
                return Err(Stop::Budget);
            }
            self.budget -= 1;
        }
        let db = self.db;
        let span = db.stmts[s].span;
        match &db.stmts[s].kind {
            StmtKind::Null => Ok(Flow::fall(st)),
            StmtKind::Assert { .. } | StmtKind::Report { .. } => {
                self.dropped += 1;
                if self.top {
                    let mut lsps = Vec::new();
                    db.stmt_exprs(s, &mut |x, _| db.reads(x, &mut lsps));
                    let mut reads = std::mem::take(&mut self.reads);
                    self.lsps_into(&lsps, &mut reads);
                    self.reads = reads;
                }
                Ok(Flow::fall(st))
            }
            StmtKind::VarAssign { target, rhs } => match rhs {
                Rhs::Simple(v) => {
                    let d = self.expr(*v, &st)?;
                    self.assign(&mut st, target.expr, d)?;
                    self.track(&mut st, target.expr, Some(*v));
                    Ok(Flow::fall(st))
                }
                Rhs::Conditional(alts) => {
                    self.track(&mut st, target.expr, None);
                    let conds: Vec<Option<ExprId>> = alts.iter().map(|a| a.1).collect();
                    self.cond_assign(st, &conds, span, &mut |cx, st, i| {
                        let d = cx.expr(alts[i].0, st)?;
                        cx.assign(st, target.expr, d)
                    })
                }
                Rhs::Selected { expr, alts, .. } => {
                    self.track(&mut st, target.expr, None);
                    self.sel_assign(st, *expr, alts.len(), span, &mut |cx, st, i| {
                        let d = cx.expr(alts[i].0, st)?;
                        cx.assign(st, target.expr, d)
                    })
                }
            },
            StmtKind::SignalAssign { target, delay, rhs } => {
                self.signal_assign(st, target.expr, delay, rhs, span)
            }
            StmtKind::If { branches, else_ } => self.if_stmt(st, branches, else_.as_deref(), span),
            StmtKind::Case { expr, alts, .. } => {
                let ctl0 = st.ctl.clone();
                let d = self.expr(*expr, &st)?;
                let mut ctl = (*ctl0).clone();
                ctl.union(&self.flat(&d));
                let ctl = Rc::new(ctl);
                let mut flows = Vec::new();
                for (_, body) in alts.iter() {
                    let mut b = st.clone();
                    b.ctl = ctl.clone();
                    flows.push(self.stmts(body, b)?);
                }
                Ok(self.merge(flows, ctl0, ctl, span))
            }
            StmtKind::Loop { scheme, body, .. } => self.loop_stmt(s, st, scheme, body),
            StmtKind::Next { loop_, when } | StmtKind::Exit { loop_, when } => {
                let next = matches!(db.stmts[s].kind, StmtKind::Next { .. });
                let mut f = Flow::default();
                let mut taken = st.clone();
                if let Some(c) = when {
                    let d = self.expr(*c, &st)?;
                    let mut ctl = (*st.ctl).clone();
                    ctl.union(&self.flat(&d));
                    taken.ctl = Rc::new(ctl.clone());
                    st.ctl = Rc::new(ctl);
                    f.fall = Some(st);
                }
                if next {
                    f.nexts.push((*loop_, taken));
                } else {
                    f.exits.push((*loop_, taken));
                }
                Ok(f)
            }
            StmtKind::Wait { .. } => {
                out(codes::WAIT, span, "a wait that is not `wait until <edge>;`")
            }
            StmtKind::ProcCall(c) => {
                if c.method_of.is_some() {
                    return out(codes::TESTBENCH, span, "a method of a protected object");
                }
                let info = self.callee(c.callee);
                // Analysis rejects a call that can wait in a process with a sensitivity list
                // (E1801); one without is behavioural when it calls one (`waits_behavioural`).
                if info.waits {
                    return bug(
                        span,
                        format!("a call of `{}`, which waits", db.decls[c.callee].name),
                    );
                }
                let mut st = self.proc_call(st, c.callee, &c.args, span)?;
                // A procedure may write what it is given, also through a name it was given.
                st.known = Rc::default();
                Ok(Flow::fall(st))
            }
            StmtKind::Force { .. } | StmtKind::Release { .. } => {
                out(codes::FORCE, span, "`force` or `release`")
            }
            StmtKind::SeqBlock { body, scope } => {
                self.decls(*scope)?;
                self.stmts(body, st)
            }
            // Analysis allows `return` in subprograms only, and no concurrent statement in a
            // sequence of statements.
            StmtKind::Return(_) => bug(span, "a `return` outside a subprogram"),
            _ => bug(span, "a concurrent statement in a sequence of statements"),
        }
    }

    fn if_stmt(
        &mut self,
        st: St,
        branches: &[(ExprId, Box<[StmtId]>)],
        else_: Option<&[StmtId]>,
        span: Span,
    ) -> R<Flow> {
        let ctl0 = st.ctl.clone();
        let mut acc = (*ctl0).clone();
        let mut flows = Vec::new();
        let mut covered = false;
        for (c, body) in branches {
            // A condition elaboration knows (a generic) picks its branch statically; the
            // variables known are those before the statement, not those of a branch before.
            self.vknown = st.known.clone();
            match self.static_bool(*c) {
                Some(false) => continue,
                Some(true) => covered = true,
                None => {
                    let d = self.expr(*c, &st)?;
                    acc.union(&self.flat(&d));
                }
            }
            let mut b = st.clone();
            b.ctl = Rc::new(acc.clone());
            flows.push(self.stmts(body, b)?);
            if covered {
                break;
            }
        }
        let acc = Rc::new(acc);
        if !covered {
            let mut b = st;
            b.ctl = acc.clone();
            flows.push(match else_ {
                Some(e) => self.stmts(e, b)?,
                None => Flow::fall(b),
            });
        }
        Ok(self.merge(flows, ctl0, acc, span))
    }

    /// Join the flows of the branches of one statement. After it, what runs depends on the
    /// conditions only when a branch left the enclosing loop early.
    fn merge(&mut self, flows: Vec<Flow>, ctl0: Rc<ESet>, conds: Rc<ESet>, span: Span) -> Flow {
        let mut out = Flow::default();
        let mut falls = Vec::new();
        let mut early = false;
        for f in flows {
            match f.fall {
                Some(s) => falls.push(s),
                None => early = true,
            }
            early |= !f.exits.is_empty() || !f.nexts.is_empty();
            out.exits.extend(f.exits);
            out.nexts.extend(f.nexts);
        }
        if let Some(mut j) = self.join_all(falls, span) {
            // What the branches depend on, from conditions of inner early exits too.
            j.ctl = if early {
                let mut c = (*j.ctl).clone();
                c.union(&conds);
                Rc::new(c)
            } else {
                ctl0
            };
            out.fall = Some(j);
        }
        out
    }

    fn join_all(&mut self, states: Vec<St>, span: Span) -> Option<St> {
        let mut it = states.into_iter();
        let mut acc = it.next()?;
        let mut any = acc.def.clone();
        for s in it {
            any.union(&s.def);
            acc = join(acc, s);
        }
        if self.comb {
            let lost = any.minus(&acc.def);
            if !lost.is_empty() {
                self.lost.push((lost, span));
            }
        }
        Some(acc)
    }

    fn loop_stmt(&mut self, s: StmtId, st: St, scheme: &LoopScheme, body: &[StmtId]) -> R<Flow> {
        let db = self.db;
        let span = db.stmts[s].span;
        let ctl0 = st.ctl.clone();
        // Static `for`: one pass per value, with the parameter known.
        if let LoopScheme::For { param, range } = scheme
            && self.comb
            && self.budget > 0
            && let Some(b) = self.range_bounds(range)
        {
            let enumd = (|| -> R<Option<St>> {
                self.enumerating += 1;
                let mut cur = Some(st.clone());
                let mut exits = Vec::new();
                let n = b.len();
                let enumerated = self.enum_param(*param);
                for i in 0..n {
                    let Some(s0) = cur.take() else { break };
                    if self.budget == 0 {
                        return Err(Stop::Budget);
                    }
                    self.budget -= 1;
                    let v = match b.dir {
                        Direction::To => b.left + i as i64,
                        Direction::Downto => b.left - i as i64,
                    };
                    // An enumeration parameter is its position, as elaboration folds it.
                    let v = match (enumerated, u32::try_from(v)) {
                        (true, Ok(p)) => ConstValue::Enum(p),
                        _ => ConstValue::Int(v),
                    };
                    self.loopv.insert(*param, v);
                    let f = self.stmts(body, s0)?;
                    let mut back = Vec::new();
                    back.extend(f.fall);
                    for (l, x) in f.nexts {
                        if l == s {
                            back.push(x);
                        } else {
                            return Ok(None);
                        }
                    }
                    for (l, x) in f.exits {
                        if l == s {
                            exits.push(x);
                        } else {
                            return Ok(None);
                        }
                    }
                    cur = self.join_all(back, span);
                }
                exits.extend(cur);
                Ok(self.join_all(exits, span))
            })();
            // The parameter is unbound on every way out, or a fallback would fold with it.
            self.loopv.remove(param);
            self.enumerating -= 1;
            match enumd {
                Ok(Some(mut j)) => {
                    j.ctl = ctl0;
                    return Ok(Flow::fall(j));
                }
                Ok(None) => {}
                Err(Stop::Budget) if self.enumerating == 0 => self.budget = 0,
                Err(e) => return Err(e),
            }
        }
        // Otherwise: rounds without the parameter's value until the dependences settle.
        let mut cond = ESet::default();
        match scheme {
            LoopScheme::While(c) => {
                let d = self.expr(*c, &st)?;
                cond = self.flat(&d);
            }
            LoopScheme::For { range, .. } => {
                if let RangeC::Explicit { left, right, .. } = range {
                    for x in [*left, *right] {
                        let d = self.expr(x, &st)?;
                        cond.union(&self.flat(&d));
                    }
                }
            }
            LoopScheme::Forever => {}
        }
        let forever = matches!(scheme, LoopScheme::Forever);
        // The values the body changes are not known in it: a clocked process walks it once.
        let mut st = st;
        if !st.known.is_empty() {
            let mut stmts = Vec::new();
            deep(db, body, &mut stmts);
            for s in stmts {
                match &db.stmts[s].kind {
                    StmtKind::VarAssign { target, .. } => {
                        let mut x = target.expr;
                        while let ExprKind::Index { prefix, .. }
                        | ExprKind::Slice { prefix, .. }
                        | ExprKind::Field { prefix, .. } = &db.exprs[x].kind
                        {
                            x = *prefix;
                        }
                        db.walk_expr(x, &mut |y| {
                            if let ExprKind::Obj(d) = &db.exprs[y].kind
                                && st.known.contains_key(d)
                            {
                                Rc::make_mut(&mut st.known).remove(d);
                            }
                        });
                    }
                    StmtKind::ProcCall(_) => st.known = Rc::default(),
                    _ => {}
                }
            }
        }
        let entry = st;
        let mut cur = entry.clone();
        let rounds = if self.comb { ROUNDS } else { 1 };
        let mut exits = Vec::new();
        for round in 0..rounds {
            let mut b = cur.clone();
            let mut ctl = (*b.ctl).clone();
            ctl.union(&cond);
            if let LoopScheme::While(c) = scheme {
                let d = self.expr(*c, &b)?;
                ctl.union(&self.flat(&d));
            }
            b.ctl = Rc::new(ctl);
            let f = self.stmts(body, b)?;
            let mut back = vec![entry.clone()];
            back.extend(f.fall);
            exits.clear();
            for (l, x) in f.nexts {
                if l == s {
                    back.push(x);
                } else {
                    return out(codes::LOOP_BOUND, span, "a `next` of an outer loop");
                }
            }
            for (l, x) in f.exits {
                if l == s {
                    exits.push(x);
                } else {
                    return out(codes::LOOP_BOUND, span, "an `exit` of an outer loop");
                }
            }
            let next = self
                .join_all(back, span)
                .expect("the entry state is one of them");
            if next.vals == cur.vals && next.ctl == cur.ctl && next.known == cur.known {
                break;
            }
            if round + 1 == rounds && self.comb {
                return out(
                    codes::LOOP_BOUND,
                    span,
                    "a loop whose bounds are not known during elaboration",
                );
            }
            cur = next;
        }
        if !forever {
            exits.push(cur);
        }
        match self.join_all(exits, span) {
            Some(mut j) => {
                j.ctl = ctl0;
                Ok(Flow::fall(j))
            }
            None => out(codes::LOOP_BOUND, span, "a loop without an exit"),
        }
    }

    fn signal_assign(
        &mut self,
        st: St,
        target: ExprId,
        delay: &yavs_sem::hir::Delay,
        rhs: &Rhs<Waveform>,
        span: Span,
    ) -> R<Flow> {
        match rhs {
            Rhs::Simple(w) => {
                let mut st = st;
                self.signal_assign_value(&mut st, target, delay, w, span)?;
                Ok(Flow::fall(st))
            }
            Rhs::Conditional(alts) => {
                let conds: Vec<Option<ExprId>> = alts.iter().map(|a| a.1).collect();
                self.cond_assign(st, &conds, span, &mut |cx, st, i| {
                    cx.signal_assign_value(st, target, delay, &alts[i].0, span)
                })
            }
            Rhs::Selected { expr, alts, .. } => {
                self.sel_assign(st, *expr, alts.len(), span, &mut |cx, st, i| {
                    cx.signal_assign_value(st, target, delay, &alts[i].0, span)
                })
            }
        }
    }

    /// `v1 when c1 else v2 when c2 else ...` as an `if` chain: `f` assigns alternative `i`.
    fn cond_assign(
        &mut self,
        st: St,
        conds: &[Option<ExprId>],
        span: Span,
        f: &mut dyn FnMut(&mut Self, &mut St, usize) -> R<()>,
    ) -> R<Flow> {
        let ctl0 = st.ctl.clone();
        let mut acc = (*ctl0).clone();
        let mut states = Vec::new();
        let mut covered = false;
        for (i, c) in conds.iter().enumerate() {
            let mut b = st.clone();
            match c.map(|c| (c, self.static_bool(c))) {
                Some((_, Some(false))) => continue,
                Some((_, Some(true))) | None => covered = true,
                Some((c, None)) => {
                    let d = self.expr(c, &st)?;
                    acc.union(&self.flat(&d));
                }
            }
            b.ctl = Rc::new(acc.clone());
            f(self, &mut b, i)?;
            states.push(b);
            if covered {
                break;
            }
        }
        if !covered {
            let mut b = st;
            b.ctl = Rc::new(acc);
            states.push(b);
        }
        let mut j = self.join_all(states, span).expect("one alternative");
        j.ctl = ctl0;
        Ok(Flow::fall(j))
    }

    fn sel_assign(
        &mut self,
        st: St,
        sel: ExprId,
        n: usize,
        span: Span,
        f: &mut dyn FnMut(&mut Self, &mut St, usize) -> R<()>,
    ) -> R<Flow> {
        let ctl0 = st.ctl.clone();
        let d = self.expr(sel, &st)?;
        let mut ctl = (*ctl0).clone();
        ctl.union(&self.flat(&d));
        let ctl = Rc::new(ctl);
        let mut states = Vec::new();
        for i in 0..n {
            let mut b = st.clone();
            b.ctl = ctl.clone();
            f(self, &mut b, i)?;
            states.push(b);
        }
        let mut j = self.join_all(states, span).unwrap_or(st);
        j.ctl = ctl0;
        Ok(Flow::fall(j))
    }

    fn signal_assign_value(
        &mut self,
        st: &mut St,
        target: ExprId,
        delay: &yavs_sem::hir::Delay,
        w: &Waveform,
        span: Span,
    ) -> R<()> {
        use yavs_sem::hir::Delay;
        match delay {
            Delay::Transport => {
                return out(codes::DELAY, span, "a `transport` delay");
            }
            Delay::Inertial { reject: Some(_) } => {
                return out(codes::DELAY, span, "a `reject` limit");
            }
            _ => {}
        }
        let Waveform::Elements(els) = w else {
            // `unaffected`: no assignment on this path.
            return Ok(());
        };
        let [el] = &els[..] else {
            return out(codes::DELAY, span, "a waveform of several elements");
        };
        if el.after.is_some_and(|a| {
            !matches!(
                self.fold(a),
                Some(ConstValue::Phys(0)) | Some(ConstValue::Int(0))
            )
        }) {
            return out(codes::DELAY, span, "a delayed assignment (`after`)");
        }
        let Some(v) = el.value else {
            return out(codes::GUARDED, span, "a `null` waveform");
        };
        let d = self.expr(v, st)?;
        self.assign(st, target, d)
    }

    /// Assign `val` to the target `t` (a name, or an aggregate of names).
    fn assign(&mut self, st: &mut St, t: ExprId, val: Dv) -> R<()> {
        let db = self.db;
        if let ExprKind::Aggregate { assocs, .. } = &db.exprs[t].kind {
            let s = Rc::new(self.flat(&val));
            for a in assocs.iter() {
                self.assign(st, a.value, Dv::Flat(s.clone()))?;
            }
            return Ok(());
        }
        // Analysis takes names and aggregates of names as targets only; an external name stops
        // at E4138 first.
        let Some(p) = self.place(t, st)? else {
            return bug(db.exprs[t].span, "an assignment target of this form");
        };
        let mut extra = (*st.ctl).clone();
        extra.union(&p.idx);
        let extra = Rc::new(extra);
        let total: u32 = p.parts.iter().map(|x| x.2 - x.1).sum();
        // The new dependences, as runs over the target's elements.
        let runs: Vec<(u32, Dep)> = match val {
            Dv::Runs(r) if p.precise && r.iter().map(|x| x.0).sum::<u32>() == total => {
                r.iter().map(|(n, d)| (*n, with_set(d, &extra))).collect()
            }
            v => {
                let mut s = self.flat(&v);
                s.union(&extra);
                vec![(total, Dep::set(Rc::new(s)))]
            }
        };
        // Walk the target's parts and the value's runs side by side.
        let (mut ri, mut used) = (0usize, 0u32);
        for &(k, a, z) in &p.parts {
            let mut at = a;
            while at < z && ri < runs.len() {
                let (n, d) = &runs[ri];
                let take = (z - at).min(n - used);
                set_vals(st, k, at, at + take, shift(d, used), p.precise);
                at += take;
                used += take;
                if used == *n {
                    ri += 1;
                    used = 0;
                }
            }
            if p.precise {
                st.def.add(k, a, z);
            }
            if p.sig {
                self.sig_any.add(k, a, z);
            } else {
                self.var_any.add(k, a, z);
            }
        }
        Ok(())
    }

    fn proc_call(
        &mut self,
        mut st: St,
        callee: DeclId,
        args: &[yavs_sem::hir::Arg],
        span: Span,
    ) -> R<St> {
        let db = self.db;
        let c = self.callee(callee);
        self.callee_ok(callee, &c, span)?;
        let Some(sp) = db.decls[callee].kind.subprogram() else {
            return bug(span, "a call of something that is not a subprogram");
        };
        // Reads: every `in` and `inout` actual, and the signals the procedure reads itself.
        let mut ins = ESet::default();
        for a in args {
            let mode = sp
                .params
                .get(usize::from(a.formal))
                .map_or(Mode::In, |p| p.mode);
            if matches!(mode, Mode::In | Mode::Inout) {
                for x in actual_exprs(&a.actual) {
                    let d = self.expr(x, &st)?;
                    ins.union(&self.flat(&d));
                }
            }
        }
        for &d in &c.sig_reads {
            let Some(o) = self.e.signal(self.p.scope, d) else {
                return out(
                    codes::CALL,
                    span,
                    format!(
                        "a call of `{}`, which reads a signal yavs cannot place in the design",
                        db.decls[callee].name
                    ),
                );
            };
            for (k, a, z) in self.whole(o) {
                self.reads.add(k, a, z);
                ins.add(k, a, z);
            }
        }
        let ins = Rc::new(ins);
        // Writes: `out` and `inout` actuals, each depending on everything read.
        for a in args {
            let Some(p) = sp.params.get(usize::from(a.formal)) else {
                continue;
            };
            if !matches!(p.mode, Mode::Out | Mode::Inout | Mode::Buffer) {
                continue;
            }
            let x = match &a.actual {
                Actual::Expr(x) => *x,
                Actual::Converted { actual, .. } => *actual,
                _ => {
                    return out(
                        codes::ANALYSIS,
                        span,
                        "an `out` or `inout` parameter associated part by part: pass one whole variable or \
                         signal to it",
                    );
                }
            };
            let before = st.def.clone();
            self.assign(&mut st, x, Dv::Flat(ins.clone()))?;
            // Assigned for sure only when every call assigns the formal.
            let whole = c
                .must_out
                .get(usize::from(a.formal))
                .copied()
                .unwrap_or(false)
                || p.mode == Mode::Inout;
            if !whole {
                st.def = before;
            }
        }
        Ok(st)
    }

    // -- names for messages -------------------------------------------------------------------

    /// Element `x` of signal leaf `k`, as VHDL writes it where it can.
    fn elem_name(&self, k: u64, x: u32) -> String {
        if k & VAR != 0 {
            return format!("element {x} of a variable");
        }
        let e = self.e;
        let l = LeafId(k as u32);
        let leaf = e.nets.leaf(l);
        let name = e.leaf_name(l);
        if leaf.ty.len == 1 {
            return name;
        }
        // The index of a one-dimensional array leaf of scalars.
        let mut sh = &e.nets.obj(leaf.obj).shape;
        for &f in &leaf.path {
            if let Shape::Record { fields, .. } = sh {
                sh = &fields[f as usize].1;
            }
        }
        if let Shape::Array { dims, elem, .. } = sh
            && let [b] = &dims[..]
            && matches!(**elem, Shape::Scalar { .. })
            && b.len() == u64::from(leaf.ty.len)
        {
            let p = if leaf.ty.elem == Elem::Logic {
                u64::from(leaf.ty.len - 1 - x)
            } else {
                u64::from(x)
            };
            let i = match b.dir {
                Direction::To => b.left + p as i64,
                Direction::Downto => b.left - p as i64,
            };
            return format!("{name}({i})");
        }
        format!("element {x} of {name}")
    }

    fn var_name(&self, d: Option<DeclId>, k: u64, x: u32) -> String {
        let Some(d) = d else {
            return "a variable".into();
        };
        let name = &self.db.decls[d].name;
        if k & (1 << 40) != 0 {
            return format!("the variable `{name}`");
        }
        let l = LeafId((k & !VAR) as u32);
        let leaf = self.vars.leaf(l);
        if leaf.ty.len == 1 {
            format!("the variable `{name}`")
        } else {
            format!("element {x} of the variable `{name}`")
        }
    }
}

/// `d` for the elements from its own element `off` on.
fn shift(d: &Dep, off: u32) -> Dep {
    Dep {
        ident: d.ident.map(|(k, b)| (k, b + off)),
        set: d.set.clone(),
    }
}

fn with_set(d: &Dep, extra: &Rc<ESet>) -> Dep {
    if extra.is_empty() {
        return d.clone();
    }
    if d.set.is_empty() {
        return Dep {
            ident: d.ident,
            set: extra.clone(),
        };
    }
    let mut s = (*d.set).clone();
    s.union(extra);
    Dep {
        ident: d.ident,
        set: Rc::new(s),
    }
}

/// Set (`strong`) or widen the dependences of elements `a .. z` of `k`.
fn set_vals(st: &mut St, k: u64, a: u32, z: u32, d: Dep, strong: bool) {
    let segs = st.vals.entry(k).or_default();
    let mut out: Vec<(u32, u32, Dep)> = Vec::with_capacity(segs.len() + 2);
    let mut covered = ESet::default();
    for (lo, hi, old) in segs.drain(..) {
        if hi <= a || lo >= z {
            out.push((lo, hi, old));
            continue;
        }
        if lo < a {
            out.push((lo, a, old.clone()));
        }
        if hi > z {
            out.push((z, hi, shift(&old, z - lo)));
        }
        if !strong {
            // The overlap keeps the old dependences too.
            let (x, y) = (lo.max(a), hi.min(z));
            let o = shift(&old, x - lo);
            let n = shift(&d, x - a);
            out.push((x, y, union_dep(&o, &n, y - x)));
            covered.add(0, x, y);
        }
    }
    if strong {
        out.push((a, z, d));
    } else {
        let gaps = {
            let mut g = ESet::default();
            g.add(0, a, z);
            g.minus(&covered)
        };
        for (_, x, y) in gaps.iter() {
            out.push((x, y, shift(&d, x - a)));
        }
    }
    out.sort_by_key(|r| r.0);
    *segs = out;
}

/// Element by element the union of two dependences of runs of `n` elements.
fn union_dep(a: &Dep, b: &Dep, n: u32) -> Dep {
    if a == b {
        return a.clone();
    }
    if a.ident == b.ident {
        let mut s = (*a.set).clone();
        s.union(&b.set);
        return Dep {
            ident: a.ident,
            set: Rc::new(s),
        };
    }
    let mut s = (*a.set).clone();
    s.union(&b.set);
    for (k, x) in [a.ident, b.ident].into_iter().flatten() {
        s.add(k, x, x + n);
    }
    Dep::set(Rc::new(s))
}

/// Runs sorted, each of at least one element, none overlapping the next.
fn tidy(s: &[(u32, u32, Dep)]) -> bool {
    s.iter().all(|r| r.0 < r.1) && s.windows(2).all(|w| w[0].1 <= w[1].0)
}

/// `mine` after `set_vals(.., false)` of every run of `segs`, one after the other (lists that are not
/// [`tidy`]).
fn widen_each(mine: &[(u32, u32, Dep)], segs: Vec<(u32, u32, Dep)>) -> Vec<(u32, u32, Dep)> {
    let mut st = St::default();
    st.vals.insert(0, mine.to_vec());
    for (lo, hi, d) in segs {
        set_vals(&mut st, 0, lo, hi, d, false);
    }
    st.vals.remove(&0).unwrap_or_default()
}

/// [`widen_each`] of two [`tidy`] lists in one pass: the same runs (cut at every boundary of
/// either, never joined), the same dependences.
fn widen_all(mine: &[(u32, u32, Dep)], segs: &[(u32, u32, Dep)]) -> Vec<(u32, u32, Dep)> {
    let mut out = Vec::with_capacity(mine.len() + 2 * segs.len());
    let mut rest = mine.iter().cloned();
    let mut front = rest.next();
    for (a, z, d) in segs {
        let (a, z) = (*a, *z);
        while let Some(f) = front.take_if(|f| f.1 <= a) {
            out.push(f);
            front = rest.next();
        }
        // The first element of `a .. z` no run of `mine` covered so far.
        let mut p = a;
        while let Some((lo, hi, old)) = front.take_if(|f| f.0 < z) {
            if lo < a {
                out.push((lo, a, old.clone()));
            }
            let (x, y) = (lo.max(a), hi.min(z));
            if x > p {
                out.push((p, x, shift(d, p - a)));
            }
            out.push((
                x,
                y,
                union_dep(&shift(&old, x - lo), &shift(d, x - a), y - x),
            ));
            p = y;
            front = if hi > z {
                Some((z, hi, shift(&old, z - lo)))
            } else {
                rest.next()
            };
        }
        if p < z {
            out.push((p, z, shift(d, p - a)));
        }
    }
    out.extend(front);
    out.extend(rest);
    out
}

/// The join of the states at the end of two paths: dependences of either, elements assigned on
/// both.
fn join(a: St, b: St) -> St {
    if a == b {
        return a;
    }
    let mut vals = a.vals;
    for (k, segs) in b.vals {
        match vals.get(&k) {
            None => {
                vals.insert(k, segs);
            }
            Some(mine) if *mine == segs => {}
            Some(mine) if tidy(mine) && tidy(&segs) => {
                let merged = widen_all(mine, &segs);
                vals.insert(k, merged);
            }
            Some(mine) => {
                let merged = widen_each(mine, segs);
                vals.insert(k, merged);
            }
        }
    }
    let ctl = if a.ctl == b.ctl {
        a.ctl
    } else {
        let mut c = (*a.ctl).clone();
        c.union(&b.ctl);
        Rc::new(c)
    };
    let known = if a.known == b.known {
        a.known
    } else {
        Rc::new(
            (a.known.iter())
                .filter(|(k, v)| b.known.get(k) == Some(v))
                .map(|(k, v)| (*k, v.clone()))
                .collect(),
        )
    };
    St {
        vals,
        def: a.def.intersect(&b.def),
        ctl,
        known,
    }
}

/// The scalar variables of the process (declared in its `scope`) that only plain assignments of
/// its `body` write: none when the process declares a subprogram (which may write them) or
/// assigns through an alias; one given to a procedure is left out.
fn tracked_vars(db: &Db, scope: yavs_sem::ScopeId, body: &[StmtId]) -> HashSet<DeclId> {
    // An alias or a subprogram of the process may write a variable by another name.
    if (db.scopes[scope].order.iter()).any(|&d| {
        matches!(db.decls[d].kind, DeclKind::Alias(_)) || db.decls[d].kind.subprogram().is_some()
    }) {
        return HashSet::new();
    }
    let mut stmts = Vec::new();
    deep(db, body, &mut stmts);
    let mut vars = HashSet::new();
    let mut given = HashSet::new();
    for &s in &stmts {
        let mut xs = Vec::new();
        db.stmt_exprs(s, &mut |x, _| xs.push(x));
        for x in xs {
            let mut nested = false;
            db.walk_expr(x, &mut |y| {
                if let ExprKind::Call { callee, args, .. } = &db.exprs[y].kind {
                    nested |= db.decls[*callee].scope == scope;
                    // A function with an `out` or `inout` formal (2019) writes its actual.
                    let writes = (db.decls[*callee].kind.subprogram())
                        .is_some_and(|sp| sp.params.iter().any(|p| p.mode != Mode::In));
                    if writes {
                        for a in args.iter() {
                            for x in actual_exprs(&a.actual) {
                                db.walk_expr(x, &mut |z| {
                                    if let ExprKind::Obj(d) = &db.exprs[z].kind {
                                        given.insert(*d);
                                    }
                                });
                            }
                        }
                    }
                }
            });
            if nested {
                return HashSet::new();
            }
        }
        match &db.stmts[s].kind {
            StmtKind::VarAssign { target, .. } => match &db.exprs[target.expr].kind {
                ExprKind::Obj(d) => match &db.decls[*d].kind {
                    DeclKind::Object(o)
                        if o.class == ObjClass::Variable && db.decls[*d].scope == scope =>
                    {
                        vars.insert(*d);
                    }
                    DeclKind::Alias(_) => return HashSet::new(),
                    _ => {}
                },
                _ => {
                    let mut aliased = false;
                    db.walk_expr(target.expr, &mut |y| {
                        if let ExprKind::Obj(d) = &db.exprs[y].kind {
                            aliased |= matches!(db.decls[*d].kind, DeclKind::Alias(_));
                        }
                    });
                    if aliased {
                        return HashSet::new();
                    }
                }
            },
            StmtKind::ProcCall(c) => {
                if db.decls[c.callee].scope == scope {
                    return HashSet::new();
                }
                for a in c.args.iter() {
                    for x in actual_exprs(&a.actual) {
                        db.walk_expr(x, &mut |y| {
                            if let ExprKind::Obj(d) = &db.exprs[y].kind {
                                given.insert(*d);
                            }
                        });
                    }
                }
            }
            _ => {}
        }
    }
    vars.retain(|d| !given.contains(d));
    vars
}

/// Every statement of a list and of the lists inside it.
fn deep(db: &Db, body: &[StmtId], out: &mut Vec<StmtId>) {
    for &s in body {
        out.push(s);
        deep(db, &db.stmt_children(s), out);
    }
}

fn waveforms(r: &Rhs<Waveform>) -> Box<dyn Iterator<Item = &Waveform> + '_> {
    match r {
        Rhs::Simple(w) => Box::new(std::iter::once(w)),
        Rhs::Conditional(a) => Box::new(a.iter().map(|x| &x.0)),
        Rhs::Selected { alts, .. } => Box::new(alts.iter().map(|x| &x.0)),
    }
}

fn actual_exprs(a: &Actual) -> Vec<ExprId> {
    match a {
        Actual::Expr(x) => vec![*x],
        Actual::Converted { actual, .. } => vec![*actual],
        Actual::Individual(parts) => parts.iter().map(|p| p.1).collect(),
        _ => Vec::new(),
    }
}

/// A layout object for `shape` in `g` (leaves as elaboration makes them).
fn add_layout(
    g: &mut NetGraph,
    decl: DeclId,
    name: yavs_syntax::ast::Ident,
    scope: yavs_elab::ScopeId,
    shape: &Shape,
) -> ObjId {
    let id = ObjId(g.objs.len() as u32);
    let first = LeafId(g.leaves.len() as u32);
    for (path, leaf, n) in shape.leaves() {
        let elem = match leaf {
            Shape::Scalar { elem, .. } => *elem,
            _ => Elem::Logic,
        };
        g.leaves.push(yavs_elab::Leaf {
            obj: id,
            path,
            ty: yavs_kernel::SignalType {
                elem,
                len: u32::try_from(n).unwrap_or(u32::MAX),
            },
            float: false,
            resolution: None,
            guard: None,
            disconnect: yavs_kernel::Time::ZERO,
            implicit: None,
            implicit_more: Vec::new(),
            init: yavs_kernel::Value::Bytes(Vec::new()),
        });
    }
    let end = LeafId(g.leaves.len() as u32);
    g.objs.push(yavs_elab::SigObj {
        scope,
        decl,
        name,
        kind: yavs_elab::ObjKind::Signal,
        shape: shape.clone(),
        leaves: (first, end),
        parts: Vec::new(),
    });
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Random tidy runs over `0 .. 40`, some with an element-by-element copy.
    fn runs(seed: &mut u64) -> Vec<(u32, u32, Dep)> {
        let mut next = |n: u64| {
            *seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (*seed >> 33) % n
        };
        let mut v = Vec::new();
        let mut at = next(4) as u32;
        while at < 40 {
            let len = 1 + next(6) as u32;
            let mut s = ESet::default();
            s.add(next(3), next(8) as u32, 8 + next(8) as u32);
            let ident = (next(2) == 0).then(|| (next(3) + 10, next(20) as u32));
            v.push((
                at,
                at + len,
                Dep {
                    ident,
                    set: Rc::new(s),
                },
            ));
            at += len + next(3) as u32;
        }
        v
    }

    #[test]
    fn the_join_in_one_pass_is_the_join_run_by_run() {
        let mut seed = 7;
        for _ in 0..2000 {
            let (a, b) = (runs(&mut seed), runs(&mut seed));
            assert!(tidy(&a) && tidy(&b));
            assert_eq!(widen_all(&a, &b), widen_each(&a, b.clone()));
        }
    }
}
