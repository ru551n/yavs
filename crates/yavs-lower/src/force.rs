//! `force` and `release` (LRM 10.5.2.1, DESIGN.md "Composites"): `Force`/`Release` on the
//! whole target signal; the value is one waveform-format element (bytes or a plane block).

use yavs_diag::Span;
use yavs_ir::{ForceKind, Inst, Ty};
use yavs_sem::ExprId;
use yavs_sem::decl::{DeclKind, ObjRole};
use yavs_sem::hir::{ExprKind, Mode, Rhs, Target};
use yavs_syntax::ast::ForceMode;

use crate::R;
use crate::func::Fx;

impl Fx<'_, '_> {
    /// The force mode: as written, else `out` for a port of mode out, inout or buffer and
    /// `in` otherwise (LRM 10.5.2.1).
    fn force_kind(&self, target: ExprId, mode: Option<ForceMode>) -> ForceKind {
        match mode {
            Some(ForceMode::In) => ForceKind::In,
            Some(ForceMode::Out) => ForceKind::Out,
            None => {
                let out = match self.db.exprs[target].kind {
                    ExprKind::Obj(d) => matches!(
                        &self.db.decls[d].kind,
                        DeclKind::Object(o) if o.role == ObjRole::Port
                            && matches!(o.mode, Some(Mode::Out | Mode::Inout | Mode::Buffer))
                    ),
                    _ => false,
                };
                if out { ForceKind::Out } else { ForceKind::In }
            }
        }
    }

    pub fn force(
        &mut self,
        target: &Target,
        mode: Option<ForceMode>,
        rhs: &Rhs<ExprId>,
        span: Span,
    ) -> R<()> {
        match rhs {
            Rhs::Simple(e) => self.force_one(target, mode, *e, span),
            Rhs::Conditional(alts) => {
                let j = self.block();
                for &(e, c) in alts.iter() {
                    let next = self.block();
                    if let Some(c) = c {
                        let c = self.scalar_expr(c)?;
                        let t = self.block();
                        self.branch(c, t, next);
                        self.at(t);
                    }
                    self.force_one(target, mode, e, span)?;
                    self.jump(j);
                    self.at(next);
                }
                self.jump(j);
                self.at(j);
                Ok(())
            }
            Rhs::Selected { .. } => Err(self.cx.unsupported(span, "selected force assignments")),
        }
    }

    fn force_one(
        &mut self,
        target: &Target,
        mode: Option<ForceMode>,
        e: ExprId,
        span: Span,
    ) -> R<()> {
        let db = self.db;
        let st = db.exprs[target.expr].subtype;
        if self.is_comp(db.base(st)) {
            return Err(self.cx.unsupported(span, "force of composite signals"));
        }
        let sig = self.sig_expr(target.expr)?;
        let (tid, _, w) = self.scalar(st, span)?;
        let r = self.scalar_expr(e)?;
        self.check_scalar(r, Some(db.exprs[e].subtype), st, "", span)?;
        let logic = Ty::logic_leaf(self.fb.program().types(), tid).is_some();
        let buf = self.alloc_n(if logic { 32 } else { 8 });
        if logic {
            let bias = self.code_bias(tid);
            let code = self.enc(r, bias);
            let z = self.int(0);
            self.emit(Inst::LSet {
                base: buf,
                idx: z,
                v: code,
            });
        } else {
            self.store(buf, r, w);
        }
        let kind = self.force_kind(target.expr, mode);
        self.emit(Inst::Force {
            sig,
            kind,
            value: buf,
        });
        Ok(())
    }

    pub fn release(&mut self, target: &Target, mode: Option<ForceMode>, span: Span) -> R<()> {
        if self.is_comp(self.db.base(self.db.exprs[target.expr].subtype)) {
            return Err(self.cx.unsupported(span, "release of composite signals"));
        }
        let sig = self.sig_expr(target.expr)?;
        let kind = self.force_kind(target.expr, mode);
        self.emit(Inst::Release { sig, kind });
        Ok(())
    }
}
