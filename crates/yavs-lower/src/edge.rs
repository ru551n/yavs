//! Clock-edge idioms (#632): every one lowers to one `Inst::Edge`, which the interpreter, the
//! LLVM tier and any later backend implement. The shapes are `Db::edge_shape` (the same ones
//! the elaborator makes edge triggers of), and the level of a `wait until` (below).

use yavs_ir::{EdgeKind, Inst, Reg, RegTy, ieee};
use yavs_sem::decl::Designator;
use yavs_sem::hir::{Actual, ExprKind, Lsp, ObjRoot, StmtKind};
use yavs_sem::{DeclId, ExprId, SubtypeId};

use crate::func::Fx;
use crate::{R, ty};

impl Fx<'_, '_> {
    /// The edge test `e` is, as an `Inst::Edge` result; `None` when `e` is not an edge idiom
    /// (or the signal is not a scalar `std_ulogic`, BIT or BOOLEAN name), and is lowered as the
    /// expression it is.
    pub fn edge_test(&mut self, e: ExprId) -> R<Option<Reg>> {
        let Some((sig, kind)) = self.edge_idiom(e) else {
            return Ok(None);
        };
        self.edge_of_name(sig, kind)
    }

    /// The signal name and kind of the edge idiom `e` (`None`: lowered as the expression it is).
    fn edge_idiom(&self, e: ExprId) -> Option<(ExprId, EdgeKind)> {
        let db = self.db;
        let sh = db.edge_shape(e)?;
        // The IEEE function runs from its VHDL body under `YAVS_NO_NATIVE_IEEE` (the check that
        // the natives change nothing).
        if !sh.legacy && self.cx.opts.no_native_ieee && !self.is_predefined_edge(e) {
            return None;
        }
        // `x'event and x = '1'` names the signal twice; the one test needs it to be the same.
        if sh.other.is_some_and(|o| !db.same_name(sh.sig, o)) {
            return None;
        }
        let kind = match (sh.legacy, sh.rising) {
            (false, true) => EdgeKind::Rising,
            (false, false) => EdgeKind::Falling,
            (true, true) => EdgeKind::To1,
            (true, false) => EdgeKind::To0,
        };
        Some((sh.sig, kind))
    }

    /// The edge test an `and` conjunct of the `wait until` condition `until` is, if any: the
    /// condition is false in every cycle without that edge, so the wait may resume its process
    /// only in cycles with it (`Inst::WaitEdge`). Emitted before the `WaitOn`s. Not in a
    /// postponed process, which resumes in a later cycle than the event, and not where
    /// coverage counters count the evaluations of the condition.
    pub fn wait_edge(&mut self, until: ExprId) -> R<()> {
        if self.postponed || self.cx.opts.any_coverage() {
            return Ok(());
        }
        let Some((x, kind)) = self.edge_conjunct(until).and_then(|e| self.edge_idiom(e)) else {
            return Ok(());
        };
        if let Some(sig) = self.edge_sig(x)? {
            self.emit(Inst::WaitEdge { sig, kind });
        }
        Ok(())
    }

    /// `e`, or the first operand of the predefined `and`s it is made of that is an edge idiom,
    /// when nothing to its left can do anything but read: the wait skips the evaluations without
    /// an edge, in which a false edge test stops at its short-circuit but what stands left of it
    /// still runs (#1022).
    fn edge_conjunct(&self, e: ExprId) -> Option<ExprId> {
        let db = self.db;
        if db.edge_shape(e).is_some() {
            return Some(e);
        }
        let ExprKind::Call {
            callee,
            args,
            method_of: None,
            ..
        } = &db.exprs[e].kind
        else {
            return None;
        };
        if db.decls[*callee].name.key().as_str() != "\"and\"" {
            return None;
        }
        if !db.decls[*callee].implicit {
            // A user `"and"` that is boolean `and` (`user_and`): its boolean operand may hold the
            // edge; both operands are evaluated, so the other one must only read.
            let b = self.user_and(*callee)?;
            let mut found = None;
            for a in args {
                let Actual::Expr(x) = a.actual else {
                    return None;
                };
                if a.formal == b {
                    found = self.edge_conjunct(x);
                } else if !self.only_reads(x) {
                    return None;
                }
            }
            return found;
        }
        for a in args {
            let Actual::Expr(x) = a.actual else {
                return None;
            };
            if let Some(found) = self.edge_conjunct(x) {
                return Some(found);
            }
            if !self.only_reads(x) {
                return None;
            }
        }
        None
    }

    /// The boolean formal of the user function `callee` when its body is exactly
    /// `return (l = '1') and r;` (operands either way round) on an unconstrained `std_ulogic`
    /// `l` and a BOOLEAN `r`: pure, no effect, false whenever `r` is false.
    fn user_and(&self, callee: DeclId) -> Option<u16> {
        use yavs_sem::decl::{Body, SubprogramKind};
        use yavs_sem::implicit::Family as F;
        use yavs_sem::stat::ConstValue;
        use yavs_sem::types::Constraint;
        let db = self.db;
        let sp = db.decls[callee].kind.subprogram()?;
        let Body::Hir { scope, stmts } = &sp.body else {
            return None;
        };
        let ([l, r], SubprogramKind::Function { pure: true }) = (&*sp.params, sp.kind) else {
            return None;
        };
        let plain = |p: &yavs_sem::decl::Param| {
            p.default.is_none() && matches!(db.subtypes[p.subtype].constraint, Constraint::None)
        };
        let boolean = |st: SubtypeId| Some(db.base(st)) == db.well_known.boolean;
        let ret = sp.ret.is_some_and(|st| {
            boolean(st) && matches!(db.subtypes[st].constraint, Constraint::None)
        });
        if !ret || sp.impure_reads || !plain(l) || !plain(r) || db.scopes[*scope].order.len() != 2 {
            return None;
        }
        let boolean = |p: &yavs_sem::decl::Param| boolean(p.subtype);
        let logic = |p: &yavs_sem::decl::Param| db.is_std_ulogic(db.base(p.subtype));
        let (lp, bp, b) = match (logic(l) && boolean(r), boolean(l) && logic(r)) {
            (true, _) => (l, r, 1),
            (_, true) => (r, l, 0),
            _ => return None,
        };
        // The body's own declaration of parameter `p`.
        let is_param = |e: ExprId, p: &yavs_sem::decl::Param| match db.exprs[e].kind {
            ExprKind::Obj(d) => {
                db.decls[d].scope == *scope
                    && matches!(db.decls[d].name, Designator::Ident(i) if i.key == p.name.key)
            }
            _ => false,
        };
        let implicit2 = |e: ExprId, fam: F| -> Option<[ExprId; 2]> {
            let ExprKind::Call { callee, args, .. } = &db.exprs[e].kind else {
                return None;
            };
            let Body::Implicit(op) = &db.decls[*callee].kind.subprogram()?.body else {
                return None;
            };
            let [x, y] = &**args else { return None };
            match (op.family == fam, &x.actual, &y.actual) {
                (true, Actual::Expr(x), Actual::Expr(y)) => Some([*x, *y]),
                _ => None,
            }
        };
        let [stmt] = &**stmts else { return None };
        let StmtKind::Return(Some(e)) = &db.stmts[*stmt].kind else {
            return None;
        };
        let [x, y] = implicit2(*e, F::And)?;
        let is_one = |e: ExprId| {
            implicit2(e, F::Eq).is_some_and(|[u, v]| {
                let one = |c: ExprId| matches!(db.value(c), Some(ConstValue::Enum(3)));
                is_param(u, lp) && one(v) || one(u) && is_param(v, lp)
            })
        };
        (is_param(x, bp) && is_one(y) || is_one(x) && is_param(y, bp)).then_some(b)
    }

    /// Whether evaluating `e` has no effect and cannot fail: names, record fields, constants, predefined
    /// relations and logic and the scalar `std_logic_1164` gates and `not` (table lookups) on
    /// them. A user function may report or keep state, other `ieee` ones warn on metavalues, an
    /// index or arithmetic may fail.
    fn only_reads(&self, e: ExprId) -> bool {
        use yavs_sem::decl::Body;
        use yavs_sem::implicit::Family as F;
        let db = self.db;
        let mut ok = true;
        db.walk_expr(e, &mut |x| {
            ok &= db.exprs[x].value.is_some()
                || match &db.exprs[x].kind {
                    // A field of a record value cannot fail (a dereference is its own node).
                    ExprKind::Obj(_) | ExprKind::Const(_) | ExprKind::Field { .. } => true,
                    ExprKind::Call { callee, .. } => {
                        db.decls[*callee].kind.subprogram().is_some_and(|sp| {
                            matches!(&sp.body, Body::Implicit(op) if matches!(
                                op.family,
                                F::Eq | F::Neq | F::Lt | F::Le | F::Gt | F::Ge | F::And | F::Or
                                    | F::Nand | F::Nor | F::Xor | F::Xnor | F::Not
                            )) || matches!(
                                ieee::native_symbol(sp.mangled.as_str())
                                    .as_deref()
                                    .and_then(ieee::symbol_key)
                                    .and_then(ieee::parse),
                                Some(ieee::Op::Gate(_) | ieee::Op::Not)
                            )
                        })
                    }
                    _ => false,
                };
        });
        ok
    }

    /// `assert not (<edge conjuncts>)` (LRM 11.5): nothing to report in any cycle without that
    /// edge, so the wait of its equivalent process may resume it only on the edge.
    pub fn assert_edge(&mut self, cond: ExprId) -> R<()> {
        let db = self.db;
        if let ExprKind::Call {
            callee,
            args,
            method_of: None,
            ..
        } = &db.exprs[cond].kind
            && db.decls[*callee].implicit
            && db.decls[*callee].name.key().as_str() == "\"not\""
            && let [a] = &**args
            && let Actual::Expr(x) = a.actual
        {
            self.wait_edge(x)?;
        }
        Ok(())
    }

    /// The level `x = '1'` / `'0'` of a `wait until` that waits on `x` alone and is resumed by
    /// its events: an event ending on that level. Not in a postponed process (it may resume in
    /// a later cycle than the event) or a procedure (shared by both), and not with a timeout.
    pub fn wait_level(&mut self, until: ExprId, sens: &[Lsp], timeout: bool) -> R<Option<Reg>> {
        let db = self.db;
        if timeout || self.postponed || self.kind != yavs_ir::FuncKind::Process {
            return Ok(None);
        }
        let Some((x, pos @ (2 | 3))) = db.level_of(until) else {
            return Ok(None);
        };
        let ExprKind::Obj(d) = db.exprs[db.unaliased(x)].kind else {
            return Ok(None);
        };
        match sens {
            [l] if l.root == ObjRoot::Decl(d) && l.path.is_empty() => {}
            _ => return Ok(None),
        }
        self.edge_of_name(
            x,
            if pos == 3 {
                EdgeKind::To1
            } else {
                EdgeKind::To0
            },
        )
    }

    /// Whether `e` is the predefined `rising_edge` / `falling_edge` of BIT or BOOLEAN.
    fn is_predefined_edge(&self, e: ExprId) -> bool {
        matches!(&self.db.exprs[e].kind,
            ExprKind::Call { callee, .. } if self.db.decls[*callee].implicit)
    }

    /// `Inst::Edge` of the signal name `x`, when it is a name of one `std_ulogic`, BIT or
    /// BOOLEAN element.
    fn edge_of_name(&mut self, x: ExprId, kind: EdgeKind) -> R<Option<Reg>> {
        let Some(sig) = self.edge_sig(x)? else {
            return Ok(None);
        };
        let dst = self.reg(RegTy::I64);
        self.emit(Inst::Edge { dst, kind, sig });
        Ok(Some(dst))
    }

    /// The signal reference of the name `x`, when it is a name of one `std_ulogic`, BIT or
    /// BOOLEAN element.
    fn edge_sig(&mut self, x: ExprId) -> R<Option<Reg>> {
        let db = self.db;
        let span = db.exprs[x].span;
        let mut n = x;
        loop {
            match &db.exprs[n].kind {
                ExprKind::Obj(_) => break,
                ExprKind::Index { prefix, .. } | ExprKind::Field { prefix, .. } => n = *prefix,
                _ => return Ok(None),
            }
        }
        let base = db.base(db.exprs[x].subtype);
        if ty::is_array(db, base) {
            return Ok(None);
        }
        let tid = self.ty(base, span)?;
        if !self.is_logic(tid) {
            return Ok(None);
        }
        Ok(Some(self.sig_expr(x)?))
    }
}
