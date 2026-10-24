//! Focused expression coverage of conditions and logic expressions (docs/decisions.md
//! "Condition coverage"). The `and`/`or`/`nand`/`nor`/`xor`/`xnor`/`not` tree of a condition
//! or of the value of an assignment is read from the HIR; its other operands (relations, names,
//! calls) are the inputs. While the tree is lowered as usual, each input that gets evaluated
//! adds its value to a state register (`Inst::CoverIn`), and `Inst::CoverCond` counts the
//! covered rows when the tree is done. Evaluation order and side effects are the ones of the
//! tree without coverage: an input never evaluated (short-circuit) stays 0 in the state.

use yavs_ir::{COV_INPUTS, CovCond, CovKind, CovNode, CovRow, Inst, Ty};
use yavs_sem::ExprId;
use yavs_sem::decl::{Body, DeclKind, ObjRole};
use yavs_sem::hir::{Actual, ExprKind, Staticness};
use yavs_sem::implicit::Family;

use crate::R;
use crate::expr::V;
use crate::func::Fx;

/// A condition tree.
enum T {
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

impl Fx<'_, '_> {
    /// The scalar logic kind of expression `e`'s type: `Some(true)` for `std_ulogic`, `Some(false)`
    /// for `bit` and `boolean`.
    fn cov_scalar(&mut self, e: ExprId) -> Option<bool> {
        let db = self.db;
        let ex = &db.exprs[e];
        let tid = self.ty(db.base(ex.subtype), ex.span).ok()?;
        match self.fb.program().types().get(tid)? {
            Ty::Enum { logic: Some(k), .. } => Some(*k == yavs_ir::LogicKind::StdULogic),
            _ => None,
        }
    }

    /// The operator and operands of tree node `e`: `and` ... `not` of `bit`/`boolean` (predefined)
    /// and of `std_ulogic` (the `std_logic_1164` gates).
    fn cov_op(&self, e: ExprId) -> Option<(CovNode, Vec<ExprId>)> {
        let db = self.db;
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
            Body::Hir { .. } => {
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
            _ => return None,
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
    fn cov_const(&self, e: ExprId) -> bool {
        let db = self.db;
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
                    ObjRole::Plain | ObjRole::Deferred => {
                        o.default.is_none_or(|i| self.cov_const(i))
                    }
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
                .all(|a| matches!(a.actual, Actual::Expr(x) if self.cov_const(x))),
            ExprKind::Qualified { operand, .. }
            | ExprKind::Conv { operand, .. }
            | ExprKind::ImplicitConv { operand } => self.cov_const(*operand),
            _ => false,
        }
    }

    /// The tree below `e`, its inputs added to `leaves` (the expression, `std_ulogic`).
    fn cov_tree(&mut self, e: ExprId, leaves: &mut Vec<(ExprId, bool)>) -> Option<T> {
        let u = self.cov_scalar(e)?;
        let db = self.db;
        if !self.cov_const(e)
            && db.edge_shape(e).is_none()
            && let Some((n, ops)) = self.cov_op(e)
        {
            let a = self.cov_tree(ops[0], leaves)?;
            if n == CovNode::Not {
                return Some(T::Not(Box::new(a)));
            }
            let b = self.cov_tree(ops[1], leaves)?;
            return Some(T::Bin(n, Box::new(a), Box::new(b)));
        }
        if leaves.len() >= COV_INPUTS {
            return None;
        }
        leaves.push((e, u));
        Some(T::In(leaves.len() - 1))
    }

    /// Start the coverage of `e`, a condition or (`expr`) the value of an assignment: `None`
    /// when it has fewer than two inputs or coverage is off for it.
    fn cov_begin(&mut self, mut e: ExprId, expr: bool) -> Option<Active> {
        if !self.cov {
            return None;
        }
        let db = self.db;
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
        let whole = db.exprs[e].span;
        let (stmt, _) = self.cur?;
        if !self.cx.cov_cond_on(expr, whole.file) {
            return None;
        }
        let mut leaves = Vec::new();
        let tree = self.cov_tree(e, &mut leaves)?;
        // Inputs that are constant after elaboration (generics, constants, folded values) are
        // evaluated but get no rows; fewer than two others leave nothing to measure.
        let consts = leaves
            .iter()
            .enumerate()
            .filter(|(_, l)| self.cov_const(l.0))
            .fold(0u32, |m, (i, _)| m | 1 << i);
        if leaves.len() - (consts.count_ones() as usize) < 2 {
            return None;
        }
        let mut nodes = Vec::new();
        let mut parity: Vec<Option<bool>> = vec![None; leaves.len()];
        flatten(&tree, Some(false), &mut nodes, &mut parity);
        let mut first = None;
        for (i, &(l, _)) in leaves.iter().enumerate() {
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
                    input: db.exprs[l].span,
                    result,
                };
                let item = self
                    .cx
                    .cover_cond(self.fb.program(), stmt, whole, kind, info);
                first.get_or_insert(item);
            }
        }
        let cond = self.fb.program().cov_cond(CovCond {
            first: first?,
            inputs: leaves.len() as u8,
            consts,
            nodes,
        });
        // The first input is always evaluated and starts the state (`Inst::CoverIn`, shift 0).
        let state = self.reg(yavs_ir::RegTy::I64);
        for (i, &(l, u)) in leaves.iter().enumerate() {
            self.cov_inputs.insert(l, (state, 2 * i as u8, u));
        }
        Some(Active {
            cond,
            state,
            inputs: leaves.iter().map(|l| l.0).collect(),
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
        let a = self.cov_begin(e, false);
        let r = self.scalar_expr(e);
        self.cov_end(a);
        r
    }

    /// [`Fx::expr`] of the value of an assignment, with expression coverage when it is on.
    pub fn value_expr(&mut self, e: ExprId) -> R<V> {
        let a = self.cov_begin(e, true);
        let r = self.expr(e);
        self.cov_end(a);
        r
    }

    /// [`Fx::scalar_expr`] of the value of an assignment, with expression coverage.
    pub fn value_scalar(&mut self, e: ExprId) -> R<yavs_ir::Reg> {
        let a = self.cov_begin(e, true);
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
