//! Individual association of subprogram parameters (LRM 6.5.7.1, `Actual::Individual`): the
//! formal is a temporary of its subtype, each part copied in before the call and, for `out`
//! and `inout`, back after it. A signal formal is a reference: of the one actual of a single
//! part, or of several parts (`Inst::SigParts`, #71).
//! Conversions (`Actual::Converted`) use the same temporary: copied in through the actual
//! part for `inout`, back through the formal part.

use yavs_diag::Span;
use yavs_ir::{CmpOp, Inst, Reg, RegTy, Width};
use yavs_sem::decl::Param;
use yavs_sem::hir::{IndividualPart as Part, LspStep, Mode};
use yavs_sem::types::{Constrainedness, TypeKind};
use yavs_sem::{DeclId, ExprId};

use crate::expr::{Conv, Place};
use crate::func::{Fx, Shape, shape};
use crate::{Obj, R, Space};

impl Fx<'_, '_> {
    /// The cells of parameter `p` associated part by part.
    pub(crate) fn individual_cells(
        &mut self,
        p: &Param,
        parts: &[Part],
        conv: Conv,
        span: Span,
        cells: &mut Vec<Reg>,
    ) -> R<()> {
        let db = self.db;
        let Some(f) = p.decl else {
            return Err(self.cx.ice(span, "parameter without a declaration"));
        };
        let sh = shape(db, self.cx, p, span)?;
        if let Shape::RecSig(out) = sh {
            return self.rec_sig_parts(f, p, parts, out, span, cells);
        }
        let dope = self.formal_dope(p, parts, span)?;
        match sh {
            sh @ (Shape::Sig | Shape::SigOut) if parts.len() > 1 => {
                let out = sh == Shape::SigOut;
                let sref = self.sig_parts(f, p, parts, dope, out, span)?;
                cells.push(sref);
                if out {
                    // Each part drives through the slot it carries (`Inst::SigParts`).
                    let none = self.int(-1);
                    cells.push(none);
                }
                cells.extend(dope);
            }
            sh @ (Shape::Sig | Shape::SigOut) => {
                let [(_, e, ..)] = parts else {
                    return Err(self.cx.ice(span, "individual association without parts"));
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
                let size = self.size_of(tid, dope);
                let tmp = self.alloc_val(size, tid, Some(dope));
                // The fat pair lives in a frame slot: no register lives across the call.
                let slot = self.fb.slot(16, 8);
                let a = self.addr(Space::Frame(self.depth), slot);
                self.store(a, tmp, Width::B8);
                let d = self.offset(a, 8);
                self.store(d, dope, Width::B8);
                // Copy in for every mode: a composite formal shows the actual's value (by
                // reference, 6.5.6.2) and every element is associated (6.5.7.1).
                let obj = Obj::Array {
                    space: Space::Frame(self.depth),
                    off: slot,
                };
                self.through_formal(f, obj, parts, true, span)?;
                if p.mode != Mode::In {
                    self.writeback_parts.push((obj, f, parts.into()));
                }
                self.array_cells(conv, tmp, dope, cells);
            }
            _ => {
                return Err(self
                    .cx
                    .ice(span, "individual association of a scalar formal"));
            }
        }
        Ok(())
    }

    /// The cells of `out`/`inout` variable parameter `p` associated through conversions
    /// (6.5.7.1): a temporary of the formal's (fully constrained) subtype, which holds the
    /// actual part `copy_in` for `inout` and the subtype's default for `out`, and goes back
    /// to `actual` through the formal part `copy_out` after the call.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn converted_cells(
        &mut self,
        p: &Param,
        actual: ExprId,
        copy_in: Option<ExprId>,
        copy_out: ExprId,
        conv: Conv,
        span: Span,
        cells: &mut Vec<Reg>,
    ) -> R<()> {
        let db = self.db;
        let Some(f) = p.decl else {
            return Err(self.cx.ice(span, "parameter without a declaration"));
        };
        let what = format!("parameter {}", p.name.text.as_str().to_ascii_uppercase());
        let obj = match shape(db, self.cx, p, span)? {
            Shape::Ref => {
                let (_, _, w) = self.scalar(p.subtype, span)?;
                let off = self.fb.slot(8, 8);
                let t = self.addr(Space::Frame(self.depth), off);
                let v = match copy_in {
                    Some(e) => {
                        let v = self.scalar_expr(e)?;
                        let es = db.exprs[e].span;
                        self.check_scalar(v, Some(db.exprs[e].subtype), p.subtype, &what, es)?;
                        v
                    }
                    None => self.scalar_left(p.subtype, span)?,
                };
                self.store(t, v, w);
                cells.push(t);
                Obj::Scalar {
                    space: Space::Frame(self.depth),
                    off,
                    w,
                }
            }
            Shape::Arr => {
                // An unconstrained formal takes the conversion's bounds (5.3.2.2).
                let st = match copy_in {
                    Some(c) if db.constrainedness(p.subtype) != Constrainedness::Full => {
                        db.exprs[c].subtype
                    }
                    _ if db.constrainedness(p.subtype) != Constrainedness::Full => {
                        db.formal_conv_subtype(copy_out).unwrap_or(p.subtype)
                    }
                    _ => p.subtype,
                };
                let Some(dope) = self.subtype_dope(st, span)? else {
                    return Err(self.cx.ice(span, "converted formal without a descriptor"));
                };
                let tid = self.ty(db.base(st), span)?;
                let size = self.size_of(tid, dope);
                let tmp = self.alloc_val(size, tid, Some(dope));
                let slot = self.fb.slot(16, 8);
                let a = self.addr(Space::Frame(self.depth), slot);
                self.store(a, tmp, Width::B8);
                let d = self.offset(a, 8);
                self.store(d, dope, Width::B8);
                match copy_in {
                    // A copy, not the conversion's value: that may share the actual's data.
                    Some(e) => {
                        let v = self.expr(e)?;
                        let dims = crate::ty::array_of(db, db.base(st)).map_or(0, |x| x.1);
                        let place = Place::Array {
                            data: tmp,
                            dope,
                            off: None,
                            tid,
                            dims,
                        };
                        self.store_place(place, v, db.exprs[e].subtype, &what, span)?;
                    }
                    None => {
                        let c = db.subtypes[st].constraint.clone();
                        self.default_into(st, &c, tmp, dope, span)?;
                    }
                }
                self.array_cells(conv, tmp, dope, cells);
                Obj::Array {
                    space: Space::Frame(self.depth),
                    off: slot,
                }
            }
            _ => return Err(self.cx.ice(span, "conversion of a signal or file formal")),
        };
        let part: Part = (Box::new([]), actual, copy_out, false, false);
        self.writeback_parts.push((obj, f, Box::new([part])));
        Ok(())
    }

    /// A reference of several parts for signal formal `f` (of parameter `p`, with descriptor
    /// `dope` for an array): each part's actual with its driver slot for `out`/`inout`, at the
    /// formal's kernel offset its formal part names. The offset comes from the one walk of
    /// signal names (rule 2): the formal is bound to a reference of signal 0 from element 0
    /// while its formal parts are lowered, so a part's reference starts at its offset.
    fn sig_parts(
        &mut self,
        f: DeclId,
        p: &Param,
        parts: &[Part],
        dope: Option<Reg>,
        out: bool,
        span: Span,
    ) -> R<Reg> {
        let n = parts.len() as u32;
        let table = self.alloc_n(n * yavs_ir::SIG_PART_BYTES);
        for (k, &(_, e, ..)) in parts.iter().enumerate() {
            let (d, sref, _) = self.sig_name(e)?;
            let slot = if out {
                self.out_driver_slot(d, e)?
            } else {
                self.int(-1)
            };
            let at = self.offset(table, k as u32 * yavs_ir::SIG_PART_BYTES);
            self.store(at, sref, Width::Sig);
            let s = self.offset(at, 16);
            self.store(s, slot, Width::B8);
        }
        // The formal seen as `[signal 0, offset 0, all elements][descriptor]`.
        let total = match dope {
            Some(dope) => self.leaf_count(self.db.base(p.subtype), dope, &[], span)?,
            None => self.int(1),
        };
        let fake = self.fb.slot(24, 8);
        let a = self.addr(Space::Frame(self.depth), fake);
        let z = self.int(0);
        self.store(a, z, Width::B8);
        let l = self.offset(a, 8);
        self.store(l, total, Width::B8);
        if let Some(dope) = dope {
            let dp = self.offset(a, 16);
            self.store(dp, dope, Width::B8);
        }
        let obj = Obj::SignalRef {
            space: Space::Frame(self.depth),
            off: fake,
            out: false,
        };
        let saved = self.cx.objs.insert(f, obj);
        let r = (|| {
            for (k, &(_, _, formal, ..)) in parts.iter().enumerate() {
                let (_, fref, _) = self.sig_name(formal)?;
                let buf = self.alloc_n(16);
                self.store(buf, fref, Width::Sig);
                let first = self.load(buf, Width::B8, RegTy::I64);
                let at = self.offset(table, k as u32 * yavs_ir::SIG_PART_BYTES + 24);
                self.store(at, first, Width::B8);
            }
            Ok(())
        })();
        match saved {
            Some(o) => self.cx.objs.insert(f, o),
            None => self.cx.objs.remove(&f),
        };
        r?;
        let dst = self.reg(RegTy::Sig);
        let n = self.int(i64::from(n));
        self.emit(Inst::SigParts {
            dst,
            parts: table,
            n,
        });
        Ok(dst)
    }

    /// The cells of signal formal `f` with records in its type (`Shape::RecSig`) associated
    /// part by part: per leaf of the formal a reference of the parts that cover it, each
    /// with its driver slot (`out`), at the leaf offsets the formal parts name (found as in
    /// [`Fx::sig_parts`], the formal bound to references from element 0), then the unused
    /// slot cell and the formal's descriptor.
    fn rec_sig_parts(
        &mut self,
        f: DeclId,
        p: &Param,
        parts: &[Part],
        out: bool,
        span: Span,
        cells: &mut Vec<Reg>,
    ) -> R<()> {
        let db = self.db;
        let leaves = crate::ty::leaves(db, p.subtype);
        let t = self.ty(db.base(p.subtype), span)?;
        let desc = if yavs_ir::Ty::needs_desc(self.fb.program().types(), t) {
            match self.desc_of(p.subtype, None, span)? {
                Some(d) => Some(d),
                None => {
                    return Err(self.cx.unsupported(
                        span,
                        "individual association of an unconstrained composite signal formal",
                    ));
                }
            }
        } else {
            None
        };
        // The formal as `[signal 0, offset 0, all of leaf k]` per leaf, then its descriptor.
        let n = leaves.len() as u32;
        let fake = self.fb.slot(16 * n + 8, 8);
        let root = match desc {
            Some(d) => d,
            None => match self.desc_of(p.subtype, None, span)? {
                Some(d) => d,
                None => return Err(self.cx.ice(span, "composite signal without a descriptor")),
            },
        };
        for (k, (path, _)) in leaves.iter().enumerate() {
            let total = self.leaf_count(db.base(p.subtype), root, path, span)?;
            let a = self.addr(Space::Frame(self.depth), fake + 16 * k as u32);
            let z = self.int(0);
            self.store(a, z, Width::B8);
            let l = self.offset(a, 8);
            self.store(l, total, Width::B8);
        }
        if let Some(d) = desc {
            let a = self.addr(Space::Frame(self.depth), fake + 16 * n);
            self.store(a, d, Width::B8);
        }
        let rec: Vec<crate::RecLeaf> = leaves
            .iter()
            .enumerate()
            .map(|(k, (path, st))| crate::RecLeaf {
                path: path.clone(),
                st: *st,
                sig: k as u32,
            })
            .collect();
        let param = crate::RecParam {
            space: Space::Frame(self.depth),
            off: fake,
            leaves: n,
            out: false,
            desc: desc.is_some(),
        };
        // A mode view drives only its `out`, `inout` and `buffer` leaves, each through the
        // slot of the driver sem collected for it (`view_leaf_slot`); the others have none.
        let tree = matches!(p.mode, Mode::View { .. }).then(|| db.port_mode_tree(p.mode));
        let out = out && tree.is_none();
        // Per formal leaf: (actual reference, slot, first element) of each part over it.
        let mut per: Vec<Vec<(Reg, Reg, Reg)>> = vec![Vec::new(); leaves.len()];
        for &(_, e, formal, ..) in parts {
            let es = db.exprs[e].span;
            // The actual's leaves, in order, each with its driver slot (a view: the leaf and
            // the actual's signal, whose slot is found per formal leaf below).
            let mut actual = Vec::new();
            let mut aleaf = Vec::new();
            let mut asig = None;
            if let Some((d, steps)) = self.comp_steps(e) {
                asig = Some(d);
                let part = self.comp_select(d, &steps, es)?;
                let slot0 = if out {
                    Some(self.leaf_slot(d, e, &part, es)?)
                } else {
                    None
                };
                for (j, (al, r, _)) in part.leaves.iter().enumerate() {
                    aleaf.push(al.clone());
                    let slot = match slot0 {
                        Some(s0) => {
                            let j = self.int(j as i64);
                            self.bin(yavs_ir::IntOp::Add, s0, j)
                        }
                        None => self.int(-1),
                    };
                    actual.push((*r, slot));
                }
            } else {
                let (d, r, _) = self.sig_name(e)?;
                let slot = if out {
                    self.out_driver_slot(d, e)?
                } else {
                    self.int(-1)
                };
                actual.push((r, slot));
            }
            let saved = (
                self.cx.objs.insert(f, Obj::RecSignal(Some(param))),
                self.cx.rec_leaves.insert(f, rec.clone()),
            );
            let r = (|| {
                let Some((d, steps)) = self.comp_steps(formal) else {
                    return Err(self.cx.ice(span, "formal part is not a name of the formal"));
                };
                self.comp_select(d, &steps, db.exprs[formal].span)
            })();
            match saved.0 {
                Some(o) => self.cx.objs.insert(f, o),
                None => self.cx.objs.remove(&f),
            };
            match saved.1 {
                Some(o) => self.cx.rec_leaves.insert(f, o),
                None => self.cx.rec_leaves.remove(&f),
            };
            let fpart = r?;
            if fpart.leaves.len() != actual.len() {
                return Err(self.cx.ice(es, "formal part and actual differ in leaves"));
            }
            let lsp = match asig.and_then(|d| self.cx.objs.get(&d)) {
                Some(Obj::RecSignal(Some(_))) => None,
                _ => db.lsp(e),
            };
            for (j, ((leaf, fref, _), (aref, mut slot))) in
                fpart.leaves.iter().zip(actual).enumerate()
            {
                if let Some(tree) = &tree {
                    let driven = matches!(
                        tree.leaf_mode(&leaf.path),
                        Some(Mode::Out | Mode::Inout | Mode::Buffer)
                    );
                    if driven {
                        slot = match (asig, aleaf.get(j)) {
                            (Some(d), Some(al)) => {
                                let rel = &leaf.path[fpart.path.len().min(leaf.path.len())..];
                                self.view_leaf_slot(d, &lsp, al, rel, es)?
                            }
                            _ => {
                                let (d, ..) = self.sig_name(e)?;
                                self.out_driver_slot(d, e)?
                            }
                        };
                    }
                }
                let buf = self.alloc_n(16);
                self.store(buf, *fref, Width::Sig);
                let first = self.load(buf, Width::B8, RegTy::I64);
                let k = leaf.sig as usize;
                per.get_mut(k)
                    .ok_or_else(|| self.cx.ice(es, "formal part outside the formal"))?
                    .push((aref, slot, first));
            }
        }
        for entries in per {
            let m = entries.len() as u32;
            if m == 0 {
                return Err(self.cx.ice(span, "a leaf of the formal is not associated"));
            }
            let table = self.alloc_n(m * yavs_ir::SIG_PART_BYTES);
            for (k, (r, slot, first)) in entries.into_iter().enumerate() {
                let at = self.offset(table, k as u32 * yavs_ir::SIG_PART_BYTES);
                self.store(at, r, Width::Sig);
                let s = self.offset(at, 16);
                self.store(s, slot, Width::B8);
                let fa = self.offset(at, 24);
                self.store(fa, first, Width::B8);
            }
            let dst = self.reg(RegTy::Sig);
            let m = self.int(i64::from(m));
            self.emit(Inst::SigParts {
                dst,
                parts: table,
                n: m,
            });
            cells.push(dst);
        }
        if out || tree.is_some() {
            // Each leaf drives through the slots its parts carry.
            let none = self.int(-1);
            cells.push(none);
        }
        cells.extend(desc);
        Ok(())
    }

    /// Copy each part's actual into (`into`) or out of the temporary formal `f` at `obj`.
    /// The formal parts name `f`, so `f` is that temporary while they are lowered.
    pub(crate) fn through_formal(
        &mut self,
        f: DeclId,
        obj: Obj,
        parts: &[Part],
        into: bool,
        span: Span,
    ) -> R<()> {
        let saved = self.cx.objs.insert(f, obj);
        let r = (|| {
            let db = self.db;
            for &(_, actual, formal, ..) in parts {
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
        for (steps, ..) in parts {
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
