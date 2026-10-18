//! Individual association of subprogram parameters (LRM 6.5.7.1, `Actual::Individual`): the
//! formal is a temporary of its subtype, each part copied in before the call and, for `out`
//! and `inout`, back after it. A signal formal is a reference, so it takes one part only.

use yavs_diag::Span;
use yavs_ir::{CmpOp, Inst, Reg, RegTy, Width};
use yavs_sem::DeclId;
use yavs_sem::decl::Param;
use yavs_sem::hir::{IndividualPart as Part, LspStep, Mode};
use yavs_sem::types::TypeKind;

use crate::func::{Fx, Shape, shape};
use crate::{Obj, R, Space};

impl Fx<'_, '_> {
    /// The cells of parameter `p` associated part by part.
    pub(crate) fn individual_cells(
        &mut self,
        p: &Param,
        parts: &[Part],
        native: bool,
        span: Span,
        cells: &mut Vec<Reg>,
    ) -> R<()> {
        let db = self.db;
        let Some(f) = p.decl else {
            return Err(self.cx.ice(span, "parameter without a declaration"));
        };
        let dope = self.formal_dope(p, parts, span)?;
        match shape(db, self.cx, p, span)? {
            sh @ (Shape::Sig | Shape::SigOut) => {
                // ponytail: one part only; parts in several signals need a reference that
                // spans kernel ranges of different signals (#71).
                let [(_, e, _)] = parts else {
                    return Err(self.cx.unsupported(
                        span,
                        "individual association of a signal parameter with more than one part",
                    ));
                };
                let (d, sref, adope) = self.sig_name(*e)?;
                cells.push(sref);
                if sh == Shape::SigOut {
                    let slot = self.out_driver_slot(d, *e)?;
                    cells.push(slot);
                }
                if let Some(dope) = dope {
                    if let Some(adope) = adope {
                        let what = self.msg(&p.name.text.to_string(), yavs_ir::Dir::To);
                        self.emit(Inst::SubtypeCheck {
                            dope: adope,
                            want: dope,
                            dims: 1,
                            what,
                        });
                    }
                    cells.push(dope);
                }
            }
            Shape::Arr => {
                let Some(dope) = dope else {
                    return Err(self.cx.ice(span, "composite formal without a descriptor"));
                };
                let tid = self.ty(db.base(p.subtype), span)?;
                let (size, _) = self.layout(tid, dope);
                let tmp = self.alloc(size);
                // The fat pair lives in a frame slot: no register lives across the call.
                let slot = self.fb.slot(16, 8);
                let a = self.addr(Space::Frame(self.depth), slot);
                self.store(a, tmp, Width::B8);
                let d = self.offset(a, 8);
                self.store(d, dope, Width::B8);
                // Copy in for every mode: a composite formal shows the actual's value (by
                // reference, 6.5.6.2) and every element is associated (6.5.7.1).
                self.through_formal(f, slot, parts, true, span)?;
                if p.mode != Mode::In {
                    self.writeback_parts.push((slot, f, parts.into()));
                }
                if native {
                    cells.push(self.make_fat(tmp, dope));
                } else {
                    cells.extend([tmp, dope]);
                }
            }
            _ => {
                return Err(self
                    .cx
                    .ice(span, "individual association of a scalar formal"));
            }
        }
        Ok(())
    }

    /// Copy each part's actual into (`into`) or out of the temporary formal
    /// `f`, whose fat pair is at frame slot `slot`. The formal parts name `f`, so `f` is
    /// that temporary while they are lowered.
    pub(crate) fn through_formal(
        &mut self,
        f: DeclId,
        slot: u32,
        parts: &[Part],
        into: bool,
        span: Span,
    ) -> R<()> {
        let obj = Obj::Array {
            space: Space::Frame(self.depth),
            off: slot,
        };
        let saved = self.cx.objs.insert(f, obj);
        let r = (|| {
            let db = self.db;
            for &(_, actual, formal) in parts {
                let (from, to) = if into {
                    (actual, formal)
                } else {
                    (formal, actual)
                };
                let v = self.expr(from)?;
                let p = self.place(to)?;
                let what = format!(
                    "parameter {}",
                    db.decls[f].name.to_string().to_ascii_uppercase()
                );
                self.store_place(p, v, db.exprs[from].subtype, &what, span)?;
            }
            Ok(())
        })();
        match saved {
            Some(o) => self.cx.objs.insert(f, o),
            None => self.cx.objs.remove(&f),
        };
        r
    }

    /// The descriptor of formal `p`: its subtype's, or for an unconstrained one-dimensional
    /// array the lowest to the highest index the parts name, in the index subtype's
    /// direction (6.5.7.1, as elab does for ports). `None` for a scalar.
    fn formal_dope(&mut self, p: &Param, parts: &[Part], span: Span) -> R<Option<Reg>> {
        let db = self.db;
        if let Some(d) = self.subtype_dope(p.subtype, span)? {
            return Ok(Some(d));
        }
        let t = db.base(p.subtype);
        let TypeKind::Array { indexes, .. } = &db.types[t].kind else {
            if matches!(db.types[t].kind, TypeKind::Record { .. }) {
                return Err(self.cx.unsupported(
                    span,
                    "individual association of an unconstrained record formal",
                ));
            }
            return Ok(None);
        };
        if indexes.len() != 1 || self.is_general(t, span)? {
            return Err(self.cx.unsupported(
                span,
                "individual association of an unconstrained formal of this type",
            ));
        }
        let ix = indexes[0];
        let mut bounds: Option<(Reg, Reg)> = None;
        for (steps, _, _) in parts {
            let (l, h) = match steps.first() {
                Some(LspStep::Index(i)) if i.len() == 1 => {
                    let v = self.scalar_expr(i[0])?;
                    (v, v)
                }
                Some(LspStep::Slice(rc)) => {
                    let r = self.range(rc, span)?;
                    self.low_high(r)
                }
                _ => return Err(self.cx.ice(span, "formal part without an index or slice")),
            };
            bounds = Some(match bounds {
                None => (l, h),
                Some((lo, hi)) => {
                    let c = self.cmp(CmpOp::Lt, l, lo);
                    let lo = self.select(c, l, lo, RegTy::I64);
                    let c = self.cmp(CmpOp::Gt, h, hi);
                    let hi = self.select(c, h, hi, RegTy::I64);
                    (lo, hi)
                }
            });
        }
        let Some((lo, hi)) = bounds else {
            return Err(self.cx.ice(span, "individual association without parts"));
        };
        let r = self.scalar_bounds(ix, span)?;
        let d = self.dir_reg(r.dir);
        let z = self.int(0);
        let up = self.cmp(CmpOp::Eq, d, z);
        let left = self.select(up, lo, hi, RegTy::I64);
        let right = self.select(up, hi, lo, RegTy::I64);
        let at = self.alloc_n(24);
        let parts = self.fb.regs(&[left, right, d]);
        self.emit(Inst::MakeDope { at, parts });
        Ok(Some(at))
    }
}
