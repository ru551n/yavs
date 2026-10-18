//! VHDL-2019 reflection: `'reflect` and calls of the `std.reflection` mirror methods.
//!
//! `x'reflect` / `T'reflect` become `CallNative yavs.attr.reflect (refl, is_value, v)`, where
//! `refl` names the subtype's [`yavs_ir::Refl`] entry (built once per sem subtype, element and
//! index subtypes included) and `v` is the value (a scalar cell, or the fat pair
//! `[data, descriptor]` of an array or record) or, for a subtype mirror, the array subtype's
//! descriptor. The runtime (yavs-interp
//! `reflect.rs`) builds the mirror object from that.
//!
//! The mirror methods have native bodies; a call through a mirror access value passes the
//! access value (the mirror's handle) as `self`, then the arguments as for any native.

use yavs_diag::Span;
use yavs_ir::{Inst, Refl, ReflId, ReflKind, RegTy};
use yavs_sem::decl::Subprogram;
use yavs_sem::hir::{Arg, AttrPrefix, ExprKind};
use yavs_sem::stat::ConstValue;
use yavs_sem::types::{Direction, TypeKind};
use yavs_sem::{ExprId, SubtypeId};

use crate::R;
use crate::expr::V;
use crate::func::Fx;

impl Fx<'_, '_> {
    /// `prefix'reflect`.
    pub(crate) fn reflect_attr(&mut self, prefix: AttrPrefix, span: Span) -> R<V> {
        let db = self.db;
        let (st, value, v) = match prefix {
            AttrPrefix::Subtype(st) => {
                let v = match self.subtype_dope(st, span)? {
                    Some(d) if matches!(db.types[db.base(st)].kind, TypeKind::Array { .. }) => d,
                    _ => self.int(0),
                };
                (st, false, v)
            }
            AttrPrefix::Expr(e) => {
                let st = db.exprs[e].subtype;
                let v = match &db.types[db.base(st)].kind {
                    TypeKind::Array { .. } | TypeKind::Record { .. } => {
                        let v = self.expr(e)?;
                        let (data, desc) = self.arr(v, span)?;
                        self.make_fat(data, desc)
                    }
                    TypeKind::File { .. } | TypeKind::Protected { .. } => {
                        return Err(self
                            .cx
                            .unsupported(span, "'reflect of file and protected objects"));
                    }
                    _ => self.scalar_expr(e)?,
                };
                (st, true, v)
            }
            _ => return Err(self.cx.ice(span, "'reflect of a non-object prefix")),
        };
        let id = self.refl(st, span)?;
        let id = self.int(i64::from(id.0));
        let is_value = self.int(i64::from(value));
        let dst = self.reg(RegTy::I64);
        let native = self.fb.program().native("yavs.attr.reflect", None);
        let args = self.fb.regs(&[id, is_value, v]);
        self.emit(Inst::CallNative {
            native,
            args,
            dst: Some(dst),
        });
        Ok(V::S(dst))
    }

    /// A call of a native protected method (`std.reflection`): `self` is the mirror handle.
    pub(crate) fn native_method(
        &mut self,
        sp: &Subprogram,
        sym: &str,
        obj: ExprId,
        args: &[Arg],
        span: Span,
    ) -> R<Option<V>> {
        let ExprKind::Deref { prefix } = self.db.exprs[obj].kind else {
            return Err(self.cx.unsupported(
                span,
                "a std.reflection method called on a protected object, not through a mirror",
            ));
        };
        let mut cells = vec![self.scalar_expr(prefix)?];
        cells.extend(self.cells(sp, args, span, true)?);
        self.native(sym, None, &cells, sp.ret, span)
    }

    /// The reflection entry of `st`, built on first use.
    fn refl(&mut self, st: SubtypeId, span: Span) -> R<ReflId> {
        if let Some(&id) = self.cx.refl.get(&st) {
            return Ok(id);
        }
        let db = self.db;
        let t = db.base(st);
        let name = simple_name(self, st);
        // A placeholder first: element and designated subtypes may lead back here.
        let id = self.fb.program().reflect(Refl {
            name: name.clone(),
            kind: ReflKind::Protected,
        });
        self.cx.refl.insert(st, id);
        let kind = match &db.types[t].kind {
            TypeKind::Enum { .. } => {
                let (left, right, downto) = self.static_range(st, span)?;
                ReflKind::Enum {
                    ty: self.ty(t, span)?,
                    left,
                    right,
                    downto,
                }
            }
            TypeKind::Integer | TypeKind::UniversalInteger => {
                let (left, right, downto) = self.static_range(st, span)?;
                ReflKind::Int {
                    left,
                    right,
                    downto,
                }
            }
            TypeKind::Physical { .. } => {
                let (left, right, downto) = self.static_range(st, span)?;
                ReflKind::Physical {
                    ty: self.ty(t, span)?,
                    left,
                    right,
                    downto,
                    time: Some(t) == db.well_known.time,
                }
            }
            TypeKind::Floating | TypeKind::UniversalReal => match db.static_bounds(st) {
                Some((ConstValue::Real(left), d, ConstValue::Real(right))) => ReflKind::Float {
                    left,
                    right,
                    downto: d == Direction::Downto,
                },
                _ => {
                    return Err(self
                        .cx
                        .unsupported(span, "'reflect of a subtype with a non-static range"));
                }
            },
            TypeKind::Array { indexes, elem } => {
                let ty = self.ty(t, span)?;
                let mut index = Vec::new();
                for &i in indexes.iter() {
                    index.push(self.refl(i, span)?);
                }
                ReflKind::Array {
                    ty,
                    index,
                    elem: self.refl(*elem, span)?,
                }
            }
            TypeKind::Record { elems } => {
                let ty = self.ty(t, span)?;
                let mut out = Vec::new();
                for e in elems.iter() {
                    out.push((
                        e.name.text.as_str().to_ascii_uppercase(),
                        self.refl(e.subtype, span)?,
                    ));
                }
                ReflKind::Record { ty, elems: out }
            }
            TypeKind::Access { designated } => ReflKind::Access {
                designated: self.refl(*designated, span)?,
            },
            TypeKind::File { elem } => ReflKind::File {
                designated: self.refl(*elem, span)?,
            },
            TypeKind::Protected { .. } => ReflKind::Protected,
            TypeKind::Incomplete {
                completion: Some(c),
            } => {
                let full = db.types[*c].first_subtype;
                let r = self.refl(full, span)?;
                self.cx.refl.insert(st, r);
                return Ok(r);
            }
            _ => {
                return Err(self
                    .cx
                    .ice(span, "'reflect of a generic or incomplete type"));
            }
        };
        self.fb.program().reflect_set(id, Refl { name, kind });
        Ok(id)
    }

    /// `(left, right, downto)` of a discrete or physical subtype, as cells.
    fn static_range(&mut self, st: SubtypeId, span: Span) -> R<(i64, i64, bool)> {
        match self.db.static_bounds(st) {
            Some((l, d, r)) => match (l.as_int(), r.as_int()) {
                (Some(l), Some(r)) => Ok((l, r, d == Direction::Downto)),
                _ => Err(self.cx.ice(span, "scalar bounds of the wrong kind")),
            },
            // ponytail: scalar ranges are taken from sem's folded bounds; a range known only
            // at run time (a generic bound) needs them passed to yavs.attr.reflect.
            None => Err(self
                .cx
                .unsupported(span, "'reflect of a subtype with a non-static range")),
        }
    }
}

/// The subtype's simple name in upper case (as NVC reports it): its own name, else the type
/// mark it was derived from, else its base type's.
fn simple_name(fx: &Fx<'_, '_>, st: SubtypeId) -> String {
    let db = fx.db;
    let mut s = st;
    for _ in 0..64 {
        let sd = &db.subtypes[s];
        if let Some(n) = sd.name {
            return n.text.as_str().to_ascii_uppercase();
        }
        match sd.parent {
            Some(p) => s = p,
            None => break,
        }
    }
    db.types[db.base(st)]
        .name
        .text
        .as_str()
        .to_ascii_uppercase()
}
