//! Focused expression coverage of conditions and logic expressions (docs/decisions.md
//! "Condition coverage"). The `and`/`or`/`nand`/`nor`/`xor`/`xnor`/`not` tree of a condition
//! or of the value of an assignment is read from the HIR; its other operands (relations, names,
//! calls) are the inputs. While the tree is lowered as usual, each input that gets evaluated
//! adds its value to a state register (`Inst::CoverIn`), and `Inst::CoverCond` counts the
//! covered rows when the tree is done. Evaluation order and side effects are the ones of the
//! tree without coverage: an input never evaluated (short-circuit) stays 0 in the state.

use yavs_ir::{COV_INPUTS, CovCond, CovKind, CovNode, CovRow, Inst};
use yavs_sem::decl::{Body, DeclKind, ObjRole};
use yavs_sem::hir::{Actual, ExprKind, Staticness};
use yavs_sem::implicit::Family;
use yavs_sem::{Db, ExprId};

use crate::R;
use crate::expr::V;
use crate::func::Fx;

/// A condition tree.
pub(crate) enum T {
    In(usize),
    Not(Box<T>),
    Bin(CovNode, Box<T>, Box<T>),
}

/// A condition being lowered: its `Inst::CoverCond` operands and the inputs to unregister.
pub(crate) struct Active {
    cond: u32,
    state: yavs_ir::Reg,
    inputs: Vec<ExprId>,
}

/// An input of a decision: a leaf of its logic tree.
pub struct Leaf {
    pub expr: ExprId,
    /// A `std_ulogic` position (`'0'`/`'L'` false, `'1'`/`'H'` true), else `bit` or `boolean`.
    pub ulogic: bool,
    /// The same value in every evaluation after elaboration: no rows, no position to address.
    pub konst: bool,
}

/// The most inputs of a decision whose combinations are measured.
pub const COMB_INPUTS: usize = 8;
/// The most combinations (reachable states) of one decision that are measured.
pub const COMB_STATES: usize = 512;

/// A decision (an `if`, `elsif`, `while`, `exit`, `next` or `when` condition) or the logic
/// expression an assignment gives its target, read from the HIR: the tree of `and`, `or`,
/// `nand`, `nor`, `xor`, `xnor` and `not` and the inputs at its leaves, left to right in the
/// source.
pub struct Decision {
    /// The expression, without the `??` of a condition (3.6).
    pub whole: ExprId,
    tree: T,
    pub leaves: Vec<Leaf>,
}

/// The scalar logic kind of expression `e`'s type: `Some(true)` for `std_ulogic`, `Some(false)`
/// for `bit` and `boolean`.
fn scalar_logic(db: &Db, e: ExprId) -> Option<bool> {
    let ex = &db.exprs[e];
    match crate::ty::logic_kind(db, db.base(ex.subtype))? {
        yavs_ir::LogicKind::StdULogic => Some(true),
        yavs_ir::LogicKind::Bit | yavs_ir::LogicKind::Boolean => Some(false),
    }
}

/// The operator and operands of tree node `e`: `and` ... `not` of `bit`/`boolean` (predefined)
/// and of `std_ulogic` (the `std_logic_1164` gates).
fn cov_op(db: &Db, e: ExprId) -> Option<(CovNode, Vec<ExprId>)> {
    let ExprKind::Call {
        callee,
        args,
        method_of: None,
        implicit_cond: false,
    } = &db.exprs[e].kind
    else {
        return None;
    };
    let sp = db.decls[*callee].kind.subprogram()?;
    let node = match &sp.body {
        Body::Implicit(op) => match op.family {
            Family::And => CovNode::And,
            Family::Or => CovNode::Or,
            Family::Nand => CovNode::Nand,
            Family::Nor => CovNode::Nor,
            Family::Xor => CovNode::Xor,
            Family::Xnor => CovNode::Xnor,
            Family::Not => CovNode::Not,
            _ => return None,
        },
        // Whether the body is analysed yet or not: analysis alone must see the same tree.
        _ => {
            use yavs_ir::ieee::{Gate, Op};
            let sym = yavs_ir::ieee::native_symbol(sp.mangled.as_str())?;
            match yavs_ir::ieee::symbol_key(&sym).and_then(yavs_ir::ieee::parse)? {
                Op::Gate(Gate::And) => CovNode::And,
                Op::Gate(Gate::Or) => CovNode::Or,
                Op::Gate(Gate::Nand) => CovNode::Nand,
                Op::Gate(Gate::Nor) => CovNode::Nor,
                Op::Gate(Gate::Xor) => CovNode::Xor,
                Op::Gate(Gate::Xnor) => CovNode::Xnor,
                Op::Not => CovNode::Not,
                _ => return None,
            }
        }
    };
    let ops: Vec<ExprId> = args
        .iter()
        .map(|a| match a.actual {
            Actual::Expr(x) => Some(x),
            _ => None,
        })
        .collect::<Option<_>>()?;
    (ops.len() == if node == CovNode::Not { 1 } else { 2 }).then_some((node, ops))
}

/// Whether `e` has the same value in every evaluation after elaboration: a folded value, or
/// a static expression of literals, generics, generate parameters and constants of those.
/// Parameters of a subprogram are static to sem but new at every call, so they are not.
fn cov_const(db: &Db, e: ExprId) -> bool {
    let x = &db.exprs[e];
    if x.value.is_some() {
        return true;
    }
    if x.stat == Staticness::None {
        return false;
    }
    match &x.kind {
        ExprKind::Const(_) => true,
        ExprKind::Obj(d) => match &db.decls[*d].kind {
            DeclKind::Object(o) => match o.role {
                ObjRole::Generic | ObjRole::GenerateParam => true,
                ObjRole::Plain | ObjRole::Deferred => o.default.is_none_or(|i| cov_const(db, i)),
                _ => false,
            },
            _ => false,
        },
        ExprKind::Call {
            args,
            method_of: None,
            ..
        } => args
            .iter()
            .all(|a| matches!(a.actual, Actual::Expr(x) if cov_const(db, x))),
        ExprKind::Qualified { operand, .. }
        | ExprKind::Conv { operand, .. }
        | ExprKind::ImplicitConv { operand } => cov_const(db, *operand),
        _ => false,
    }
}

/// The tree below `e`, its inputs added to `leaves` (the expression, `std_ulogic`).
fn cov_tree(db: &Db, e: ExprId, leaves: &mut Vec<(ExprId, bool)>) -> Option<T> {
    let u = scalar_logic(db, e)?;
    if !cov_const(db, e)
        && db.edge_shape(e).is_none()
        && let Some((n, ops)) = cov_op(db, e)
    {
        let a = cov_tree(db, ops[0], leaves)?;
        if n == CovNode::Not {
            return Some(T::Not(Box::new(a)));
        }
        let b = cov_tree(db, ops[1], leaves)?;
        return Some(T::Bin(n, Box::new(a), Box::new(b)));
    }
    if leaves.len() >= COV_INPUTS {
        return None;
    }
    leaves.push((e, u));
    Some(T::In(leaves.len() - 1))
}

impl Decision {
    /// The decision of `e`, a condition or the value of an assignment; `None` when it is not
    /// a logic tree of `bit`, `boolean` or `std_ulogic`, or has too many inputs for the state.
    pub fn new(db: &Db, mut e: ExprId) -> Option<Decision> {
        // The `??` of a condition (3.6) is not an operator of it.
        while let ExprKind::Call {
            args,
            implicit_cond: true,
            ..
        } = &db.exprs[e].kind
            && let [a] = &args[..]
            && let Actual::Expr(x) = a.actual
        {
            e = x;
        }
        let mut leaves = Vec::new();
        let tree = cov_tree(db, e, &mut leaves)?;
        Some(Decision {
            whole: e,
            tree,
            leaves: leaves
                .into_iter()
                .map(|(expr, ulogic)| Leaf {
                    expr,
                    ulogic,
                    konst: cov_const(db, expr),
                })
                .collect(),
        })
    }

    /// The inputs that are constant after elaboration (bit `i`).
    pub fn consts(&self) -> u32 {
        self.leaves
            .iter()
            .enumerate()
            .filter(|(_, l)| l.konst)
            .fold(0u32, |m, (i, _)| m | 1 << i)
    }

    /// The states of the combinations the decision can show, sorted: two bits per input
    /// (0 not evaluated, 1 false, 2 true). `and`, `or`, `nand` and `nor` of `bit` and `boolean`
    /// do not evaluate their right operand when the left one decides (9.2.2); every other
    /// operator evaluates both. Evaluated leaves that read the same object agree. `Err` says
    /// why the decision has no combinations to measure.
    pub fn combinations(&self, db: &Db) -> Result<Vec<u32>, String> {
        let n = self.leaves.len();
        if n < 2 {
            return Err("it has fewer than two inputs".into());
        }
        if let Some(i) = self.leaves.iter().position(|l| l.konst) {
            return Err(format!(
                "input {} is constant after elaboration, so it has no value to combine",
                i + 1
            ));
        }
        if n > COMB_INPUTS {
            return Err(format!(
                "it has {n} inputs; combinations are measured up to {COMB_INPUTS}"
            ));
        }
        let mut out = self.states(&self.tree);
        // Leaves of one object take one value in an evaluation.
        let object = |i: usize| match &db.exprs[self.leaves[i].expr].kind {
            ExprKind::Obj(d) => Some(*d),
            _ => None,
        };
        out.retain(|&(s, _)| {
            (0..n).all(|i| {
                (0..i).all(|j| {
                    let (a, b) = (s >> (2 * i) & 3, s >> (2 * j) & 3);
                    a == 0 || b == 0 || object(i).is_none() || object(i) != object(j) || a == b
                })
            })
        });
        let mut states: Vec<u32> = out.into_iter().map(|(s, _)| s).collect();
        states.sort_unstable();
        states.dedup();
        if states.len() > COMB_STATES {
            return Err(format!(
                "it has {} combinations; the limit is {COMB_STATES}",
                states.len()
            ));
        }
        Ok(states)
    }

    /// The (state, result) pairs of the evaluations of `t`.
    fn states(&self, t: &T) -> Vec<(u32, bool)> {
        match t {
            T::In(i) => vec![(1 << (2 * i), false), (2 << (2 * i), true)],
            T::Not(a) => self.states(a).into_iter().map(|(s, v)| (s, !v)).collect(),
            T::Bin(n, a, b) => {
                let (left, right) = (self.states(a), self.states(b));
                // `ulogic` operators never short-circuit.
                let short = !self.is_ulogic(a)
                    && matches!(n, CovNode::And | CovNode::Or | CovNode::Nand | CovNode::Nor);
                let gate = |x: bool, y: bool| match n {
                    CovNode::And => x & y,
                    CovNode::Nand => !(x & y),
                    CovNode::Or => x | y,
                    CovNode::Nor => !(x | y),
                    CovNode::Xor => x ^ y,
                    _ => !(x ^ y),
                };
                let mut out = Vec::new();
                for &(sa, va) in &left {
                    // The left operand decides: `false and _`, `true or _`.
                    let decided = match n {
                        CovNode::And | CovNode::Nand => !va,
                        _ => va,
                    };
                    if short && decided {
                        out.push((sa, gate(va, va)));
                        continue;
                    }
                    out.extend(right.iter().map(|&(sb, vb)| (sa | sb, gate(va, vb))));
                }
                out.sort_unstable();
                out.dedup();
                out
            }
        }
    }

    /// Whether the leaves below `t` are `std_ulogic` (a node has operands of one type).
    fn is_ulogic(&self, t: &T) -> bool {
        match t {
            T::In(i) => self.leaves[*i].ulogic,
            T::Not(a) | T::Bin(_, a, _) => self.is_ulogic(a),
        }
    }
}

/// Where a measured expression stands: what turns its rows on, and whether it has combinations.
#[derive(Clone, Copy, PartialEq)]
enum Site {
    /// `if`, `elsif`, `when`, `while`, `exit`, `next`.
    Condition,
    /// The value of an assignment.
    Value,
    /// A `wait until`: condition rows, no combinations.
    Wait,
    /// An `assert`: condition rows of their own option (`--cov-kind=assert`), no combinations.
    Assert,
}

impl Fx<'_, '_> {
    /// Start the coverage of `e`: `None` when nothing about it is measured (coverage is off for
    /// it, or it has fewer than two inputs).
    fn cov_begin(&mut self, e: ExprId, site: Site) -> Option<Active> {
        let expr = site == Site::Value;
        if !self.cov {
            return None;
        }
        let db = self.db;
        let whole = db.exprs[e].span;
        let (stmt, _) = self.cur?;
        let file = whole.file;
        let fec_on = if site == Site::Assert {
            self.cx.cov_assert_on(file)
        } else {
            self.cx.cov_cond_on(expr, file)
        };
        let comb_on = matches!(site, Site::Condition | Site::Value) && self.cx.cov_comb_on(file);
        if !fec_on && !comb_on {
            return None;
        }
        let d = Decision::new(db, e)?;
        let whole = db.exprs[d.whole].span;
        let leaves = &d.leaves;
        // Inputs that are constant after elaboration (generics, constants, folded values) are
        // evaluated but get no rows; fewer than two others leave nothing to measure.
        let mut consts = d.consts();
        // An edge test of a wait is what resumes it: an edge trigger leaves no evaluation where
        // it is false, so its rows could never all be hit. It is evaluated and has none.
        if site == Site::Wait {
            for (i, l) in leaves.iter().enumerate() {
                if db.edge_shape(l.expr).is_some() {
                    consts |= 1 << i;
                }
            }
        }
        let fec = fec_on && leaves.len() - (consts.count_ones() as usize) >= 2;
        let states = if comb_on {
            d.combinations(db).ok()
        } else {
            None
        };
        if !fec && states.is_none() {
            return None;
        }
        let mut nodes = Vec::new();
        let mut parity: Vec<Option<bool>> = vec![None; leaves.len()];
        flatten(&d.tree, Some(false), &mut nodes, &mut parity);
        let (row_waivers, comb_waivers): (Vec<_>, Vec<_>) = self
            .cx
            .cov_waivers(db, d.whole)
            .into_iter()
            .partition(|w| w.row.is_some());
        let inputs: Vec<_> = leaves.iter().map(|l| db.exprs[l.expr].span).collect();
        let mut first = None;
        for (i, l) in leaves.iter().enumerate().filter(|_| fec) {
            if consts >> i & 1 == 1 {
                continue;
            }
            for v in 0..2usize {
                let row = (2 * i + v) as u32;
                let kind = if expr {
                    CovKind::Expression(row)
                } else {
                    CovKind::Condition(row)
                };
                let result = match parity[i] {
                    Some(inv) => char::from(b'0' + u8::from((v == 1) != inv)),
                    None => '?',
                };
                let info = CovRow {
                    whole,
                    input: db.exprs[l.expr].span,
                    result,
                    inputs: inputs.clone(),
                    waivers: row_waivers
                        .iter()
                        .filter(|w| w.row == Some(row))
                        .cloned()
                        .collect(),
                };
                let item =
                    self.cx
                        .cover_cond(self.fb.program(), stmt, kind, info, None, self.cov_parent);
                first.get_or_insert(item);
            }
        }
        let index = self.cx.cov_tree_next(self.fb.program());
        let comb = match states {
            Some(states) => {
                let mut first = None;
                for &s in &states {
                    let info = CovRow {
                        whole,
                        input: whole,
                        result: '?',
                        inputs: Vec::new(),
                        waivers: Vec::new(),
                    };
                    let item = self.cx.cover_cond(
                        self.fb.program(),
                        stmt,
                        CovKind::Combination(s),
                        info,
                        Some(index),
                        self.cov_parent,
                    );
                    first.get_or_insert(item);
                }
                Some(yavs_ir::CovComb {
                    first: first?,
                    states,
                    inputs,
                    waivers: comb_waivers,
                })
            }
            None => None,
        };
        let cond = self.cx.cov_tree(
            self.fb.program(),
            CovCond {
                first,
                inputs: leaves.len() as u8,
                consts,
                nodes,
                comb,
            },
        );
        debug_assert_eq!(cond, index);
        // The first input is always evaluated and starts the state (`Inst::CoverIn`, shift 0).
        let state = self.reg(yavs_ir::RegTy::I64);
        for (i, l) in leaves.iter().enumerate() {
            self.cov_inputs
                .insert(l.expr, (state, 2 * i as u8, l.ulogic));
        }
        Some(Active {
            cond,
            state,
            inputs: leaves.iter().map(|l| l.expr).collect(),
        })
    }

    fn cov_end(&mut self, a: Option<Active>) {
        if let Some(a) = a {
            for e in &a.inputs {
                self.cov_inputs.remove(e);
            }
            self.emit(Inst::CoverCond {
                cond: a.cond,
                state: a.state,
            });
        }
    }

    /// The value of condition `e`, with condition coverage when it is on.
    pub fn cond_expr(&mut self, e: ExprId) -> R<yavs_ir::Reg> {
        self.measured(e, Site::Condition)
    }

    /// The value of the condition of a `wait until`, with condition rows when they are on.
    pub fn wait_cond(&mut self, e: ExprId) -> R<yavs_ir::Reg> {
        self.measured(e, Site::Wait)
    }

    /// The value of the condition of an `assert`, with its rows when they are on.
    pub fn assert_cond(&mut self, e: ExprId) -> R<yavs_ir::Reg> {
        self.measured(e, Site::Assert)
    }

    fn measured(&mut self, e: ExprId, site: Site) -> R<yavs_ir::Reg> {
        let a = self.cov_begin(e, site);
        let r = self.scalar_expr(e);
        self.cov_end(a);
        r
    }

    /// [`Fx::expr`] of the value of an assignment, with expression coverage when it is on.
    pub fn value_expr(&mut self, e: ExprId) -> R<V> {
        let a = self.cov_begin(e, Site::Value);
        let r = self.expr(e);
        self.cov_end(a);
        r
    }

    /// [`Fx::scalar_expr`] of the value of an assignment, with expression coverage.
    pub fn value_scalar(&mut self, e: ExprId) -> R<yavs_ir::Reg> {
        let a = self.cov_begin(e, Site::Value);
        let r = self.scalar_expr(e);
        self.cov_end(a);
        r
    }

    /// The hook of [`Fx::expr`]: input `e` of the condition being lowered was evaluated to `v`.
    pub(crate) fn cov_input(&mut self, e: ExprId, v: V) {
        if let (Some(&(state, shift, ulogic)), V::S(src)) = (self.cov_inputs.get(&e), v) {
            self.emit(Inst::CoverIn {
                dst: state,
                // Nothing to add to for the first input; any defined register will do.
                acc: if shift == 0 { src } else { state },
                src,
                shift,
                ulogic,
            });
        }
    }
}

/// `t` in postfix order; for each input whether an `xor` lies between it and the result
/// (`None`) or else whether the path inverts (`parity`, `inv` on the way down).
fn flatten(t: &T, inv: Option<bool>, nodes: &mut Vec<CovNode>, parity: &mut [Option<bool>]) {
    match t {
        T::In(i) => {
            nodes.push(CovNode::Input(*i as u8));
            parity[*i] = inv;
        }
        T::Not(a) => {
            flatten(a, inv.map(|x| !x), nodes, parity);
            nodes.push(CovNode::Not);
        }
        T::Bin(n, a, b) => {
            let own = match n {
                CovNode::Nand | CovNode::Nor => inv.map(|x| !x),
                CovNode::Xor | CovNode::Xnor => None,
                _ => inv,
            };
            flatten(a, own, nodes, parity);
            flatten(b, own, nodes, parity);
            nodes.push(*n);
        }
    }
}
