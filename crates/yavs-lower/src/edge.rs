//! Clock-edge idioms (#632): every one lowers to one `Inst::Edge`, which the interpreter, the
//! LLVM tier and any later backend implement. The shapes are `Db::edge_shape` (the same ones
//! the elaborator makes edge triggers of), and the level of a `wait until` (below).

use yavs_ir::{EdgeKind, Inst, Reg, RegTy};
use yavs_sem::ExprId;
use yavs_sem::hir::{ExprKind, Lsp, ObjRoot};

use crate::func::Fx;
use crate::{R, ty};

impl Fx<'_, '_> {
    /// The edge test `e` is, as an `Inst::Edge` result; `None` when `e` is not an edge idiom
    /// (or the signal is not a scalar `std_ulogic`, BIT or BOOLEAN name), and is lowered as the
    /// expression it is.
    pub fn edge_test(&mut self, e: ExprId) -> R<Option<Reg>> {
        let db = self.db;
        let Some(sh) = db.edge_shape(e) else {
            return Ok(None);
        };
        // The IEEE function runs from its VHDL body under `YAVS_NO_NATIVE_IEEE` (the check that
        // the natives change nothing).
        if !sh.legacy && self.cx.opts.no_native_ieee && !self.is_predefined_edge(e) {
            return Ok(None);
        }
        // `x'event and x = '1'` names the signal twice; the one test needs it to be the same.
        if sh.other.is_some_and(|o| !db.same_name(sh.sig, o)) {
            return Ok(None);
        }
        let kind = match (sh.legacy, sh.rising) {
            (false, true) => EdgeKind::Rising,
            (false, false) => EdgeKind::Falling,
            (true, true) => EdgeKind::To1,
            (true, false) => EdgeKind::To0,
        };
        self.edge_of_name(sh.sig, kind)
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
        let ExprKind::Obj(d) = db.exprs[x].kind else {
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
        let sig = self.sig_expr(x)?;
        let dst = self.reg(RegTy::I64);
        self.emit(Inst::Edge { dst, kind, sig });
        Ok(Some(dst))
    }
}
