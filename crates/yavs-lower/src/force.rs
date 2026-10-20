//! `force` and `release` (LRM 10.5.2.1, DESIGN.md "Composites"): `Force`/`Release` on the
//! target's kernel range, or on each leaf's range for a target with records; the value is one
//! waveform-format element per range (bytes or plane blocks).

use yavs_diag::Span;
use yavs_ir::{Dir, ForceKind, Inst, Ty};
use yavs_sem::ExprId;
use yavs_sem::decl::{DeclKind, ObjRole};
use yavs_sem::hir::{Mode, ObjRoot, Rhs, Target};
use yavs_syntax::ast::ForceMode;

use crate::expr::SigPart;
use crate::func::Fx;
use crate::{R, ty};

impl Fx<'_, '_> {
    /// The force mode: as written, else `out` when the target is (part of) a port of mode
    /// out, inout or buffer and `in` otherwise (LRM 10.5.2.1).
    fn force_kind(&self, target: &Target, mode: Option<ForceMode>) -> ForceKind {
        match mode {
            Some(ForceMode::In) => ForceKind::In,
            Some(ForceMode::Out) => ForceKind::Out,
            None => {
                // An external name: the kernel knows whether it names the formal of an out port.
                if target
                    .lsps
                    .iter()
                    .any(|l| matches!(l.root, ObjRoot::External(_)))
                {
                    return ForceKind::Default;
                }
                let out = target.lsps.iter().any(|l| match l.root {
                    ObjRoot::Decl(d) => matches!(
                        &self.db.decls[d].kind,
                        DeclKind::Object(o) if o.role == ObjRole::Port
                            && matches!(o.mode, Some(Mode::Out | Mode::Inout | Mode::Buffer))
                    ),
                    _ => false,
                });
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
            Rhs::Selected {
                expr,
                matching,
                alts,
            } => {
                let sets: Vec<_> = alts.iter().map(|a| &a.1).collect();
                let blocks = self.case_blocks(*expr, *matching, &sets, span)?;
                let j = self.block();
                for (&(e, _), b) in alts.iter().zip(blocks) {
                    self.at(b);
                    self.force_one(target, mode, e, span)?;
                    self.jump(j);
                }
                self.at(j);
                Ok(())
            }
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
        let kind = self.force_kind(target, mode);
        if let Some(p) = self.force_leaves(target, span)? {
            // A part with records: each leaf gets its elements of the value (LRM 10.5.2.1
            // applies the force to every scalar subelement).
            let x = self.expr(e)?;
            let (data, vd) = self.arr(x, span)?;
            if let Some((_, dims)) = ty::array_of(db, p.t) {
                let what = self.target_name(target.expr);
                let m = self.msg(&what, Dir::To);
                self.emit(Inst::SubtypeCheck {
                    dope: vd,
                    want: p.desc,
                    dims,
                    what: m,
                });
            }
            for (leaf, sig, len) in &p.leaves {
                let rel = &leaf.path[p.path.len()..];
                let size = self.leaf_size(leaf, *len, span)?;
                let buf = self.alloc(size);
                self.leaf_of_value(leaf, rel, p.t, (data, vd), (buf, *len), span)?;
                self.emit(Inst::Force {
                    sig: *sig,
                    kind,
                    value: buf,
                });
            }
            return Ok(());
        }
        let st = db.exprs[target.expr].subtype;
        let (_, sig, dope) = self.sig_name(target.expr)?;
        let buf = match dope {
            // An array: its elements, checked against the target's bounds first.
            Some(dope) => {
                let x = self.expr(e)?;
                let (data, vd) = self.arr(x, span)?;
                let dims = ty::array_of(db, db.base(st)).map_or(1, |x| x.1);
                let what = self.target_name(target.expr);
                let m = self.msg(&what, Dir::To);
                self.emit(Inst::SubtypeCheck {
                    dope: vd,
                    want: dope,
                    dims,
                    what: m,
                });
                data
            }
            None => {
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
                buf
            }
        };
        self.emit(Inst::Force {
            sig,
            kind,
            value: buf,
        });
        Ok(())
    }

    /// The selected part of a signal with records whose target covers more than one leaf
    /// (one `Force`/`Release` per leaf); `None` for a target that is one kernel range.
    fn force_leaves(&mut self, target: &Target, span: Span) -> R<Option<SigPart>> {
        let Some((d, steps)) = self.comp_steps(target.expr) else {
            return Ok(None);
        };
        let p = self.comp_select(d, &steps, span)?;
        Ok((!matches!(p.leaves.as_slice(), [(l, _, _)] if l.path == p.path)).then_some(p))
    }

    pub fn release(&mut self, target: &Target, mode: Option<ForceMode>, span: Span) -> R<()> {
        let kind = self.force_kind(target, mode);
        let sigs = match self.force_leaves(target, span)? {
            Some(p) => p.leaves.iter().map(|l| l.1).collect(),
            None => vec![self.sig_expr(target.expr)?],
        };
        for sig in sigs {
            self.emit(Inst::Release { sig, kind });
        }
        Ok(())
    }
}
