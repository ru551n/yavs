//! Calls of `'FOREIGN` subprograms (VHPIDIRECT, ir.md 1.7): one `ForeignRef` per
//! subprogram, arguments in the shapes `yavs-interp` marshals.

use yavs_diag::Span;
use yavs_ffi::CType;
use yavs_ffi::vhdl::{self, VhdlType};
use yavs_ir::{ForeignArg, ForeignMode, ForeignRef, Inst, RegTy, Ty, VhpiModel, VhpiParam};
use yavs_sem::decl::ObjClass;
use yavs_sem::decl::Subprogram;
use yavs_sem::hir::{Arg, Mode};
use yavs_sem::stat::{ConstValue, Elems};
use yavs_sem::types::{Constraint, TypeKind};
use yavs_sem::{DeclId, ExprId, SubtypeId};

use crate::expr::V;
use crate::func::Fx;
use crate::{NOT_IMPLEMENTED, R};

/// The symbol of a `'FOREIGN` attribute value: `VHPIDIRECT [library] symbol`. As in the reference simulator, the
/// last word is the symbol and a library word is ignored (symbols come from `--load`
/// libraries); without one the subprogram's name is used, as other simulators do. `None` for any other
/// foreign kind.
pub fn vhpidirect_symbol(attr: &str, name: &str) -> Option<String> {
    let mut w = attr.split_whitespace();
    if !w.next()?.eq_ignore_ascii_case("vhpidirect") {
        return None;
    }
    Some(w.last().unwrap_or(name).to_string())
}

/// The library and model of a `'FOREIGN "VHPI library model"` attribute value (exactly three
/// words, keyword case-insensitive); `None` for any other form.
pub fn vhpi_model(attr: &str) -> Option<(String, String)> {
    let w: Vec<&str> = attr.split_whitespace().collect();
    match w[..] {
        [k, lib, model] if k.eq_ignore_ascii_case("vhpi") => Some((lib.into(), model.into())),
        _ => None,
    }
}

impl Fx<'_, '_> {
    /// A call of foreign subprogram `callee`; `None` for a procedure.
    pub(crate) fn foreign_call(
        &mut self,
        callee: DeclId,
        sp: &Subprogram,
        attr: ExprId,
        args: &[Arg],
        span: Span,
    ) -> R<Option<V>> {
        let id = match self.cx.foreigns.get(&callee) {
            Some(&id) => id,
            None => {
                let x = self.foreign_ref(callee, sp, attr, span)?;
                let id = self.fb.program().foreign(x);
                self.cx.foreigns.insert(callee, id);
                id
            }
        };
        if args.iter().enumerate().any(|(i, a)| a.formal as usize != i)
            || args.len() != sp.params.len()
        {
            return Err(self
                .cx
                .ice(span, "foreign call arguments out of formal order"));
        }
        // One cell per parameter: a value, an address, or a fat pair for composites.
        let mut cells = self.cells(sp, args, span, true)?;
        for (c, p) in cells.iter_mut().zip(&sp.params) {
            if matches!(
                self.db.types[self.db.base(p.subtype)].kind,
                TypeKind::Record { .. }
            ) {
                *c = self.load(*c, yavs_ir::Width::B8, RegTy::Ptr);
            }
        }
        let regs = self.fb.regs(&cells);
        let Some(ret) = sp.ret else {
            self.emit(Inst::CallForeign {
                foreign: id,
                args: regs,
                dst: None,
            });
            self.copy_out(span)?;
            return Ok(None);
        };
        let dst = self.reg(self.rt_of(self.db.base(ret)));
        self.emit(Inst::CallForeign {
            foreign: id,
            args: regs,
            dst: Some(dst),
        });
        self.copy_out(span)?;
        let what = format!(
            "result of {}",
            self.db.decls[callee].name.to_string().to_ascii_uppercase()
        );
        self.check_scalar(dst, None, ret, &what, span)?;
        Ok(Some(V::S(dst)))
    }

    fn foreign_ref(
        &mut self,
        callee: DeclId,
        sp: &Subprogram,
        attr: ExprId,
        span: Span,
    ) -> R<ForeignRef> {
        let db = self.db;
        let name = db.decls[callee].name.to_string();
        let text = match db.value(attr) {
            Some(ConstValue::Array(a)) => match &a.elems {
                Elems::Bytes(b) => b.iter().map(|&c| char::from(c)).collect::<String>(),
                Elems::Runs(_) => return Err(self.cx.ice(span, "foreign attribute value")),
            },
            _ => {
                return Err(self.cx.error(
                    NOT_IMPLEMENTED,
                    span,
                    format!("foreign subprogram `{name}`: the 'FOREIGN value is not static"),
                ));
            }
        };
        let vhpi = vhpi_model(&text);
        let Some(symbol) = vhpi
            .as_ref()
            .map(|(_, m)| m.clone())
            .or_else(|| vhpidirect_symbol(&text, &name))
        else {
            return Err(self.cx.error(
                NOT_IMPLEMENTED,
                span,
                format!(
                    "foreign subprogram `{name}` with 'FOREIGN \"{text}\": only VHPIDIRECT \
                     and \"VHPI library model\" subprograms are supported"
                ),
            ));
        };
        let mut params = Vec::new();
        let mut args = Vec::new();
        for p in sp.params.iter() {
            let pname = p.name.text.to_string();
            let bad = |fx: &mut Self, what: &str| {
                fx.cx.error(
                    NOT_IMPLEMENTED,
                    span,
                    format!("parameter `{pname}` of foreign subprogram `{name}`: {what}"),
                )
            };
            let copy_back = p.mode != Mode::In;
            if p.class == ObjClass::Signal || p.class == ObjClass::File {
                return Err(bad(self, "signal and file parameters are not supported"));
            }
            let t = db.base(p.subtype);
            if vhpi.is_some() {
                if matches!(
                    db.types[t].kind,
                    TypeKind::Record { .. } | TypeKind::Access { .. }
                ) {
                    return Err(bad(self, "record and access parameters of a VHPI model"));
                }
                params.push(VhpiParam {
                    name: pname.clone(),
                    mode: match p.mode {
                        Mode::In => ForeignMode::In,
                        Mode::Out => ForeignMode::Out,
                        _ => ForeignMode::InOut,
                    },
                    ty: self.ty(t, span)?,
                });
            }
            args.push(match &db.types[t].kind {
                TypeKind::Array { indexes, .. } => {
                    let full = matches!(
                        db.subtypes[p.subtype].constraint,
                        Constraint::Array { dims: Some(_), .. }
                    );
                    ForeignArg::Array {
                        ty: self.ty(t, span)?,
                        lengths: if full { 0 } else { indexes.len() as u8 },
                        copy_back,
                    }
                }
                TypeKind::Record { .. } => ForeignArg::Record {
                    ty: self.ty(t, span)?,
                    copy_back,
                },
                TypeKind::Access { .. } if !copy_back => ForeignArg::Access,
                TypeKind::Access { .. } => {
                    return Err(bad(self, "access parameters of mode out or inout"));
                }
                _ => {
                    let Some(ct) = self.ctype(p.subtype, span)? else {
                        return Err(bad(self, "the type has no C mapping"));
                    };
                    if copy_back {
                        let (_, _, w) = self.scalar(p.subtype, span)?;
                        ForeignArg::ScalarRef(ct, w)
                    } else {
                        ForeignArg::Value(ct)
                    }
                }
            });
        }
        let ret = match sp.ret {
            None => None,
            Some(r) => match self.ctype(r, span)? {
                Some(ct) => Some(ct),
                None => {
                    return Err(self.cx.error(
                        NOT_IMPLEMENTED,
                        span,
                        format!("foreign function `{name}`: only scalar results are supported"),
                    ));
                }
            },
        };
        let model = match vhpi {
            None => None,
            Some((library, model)) => Some(VhpiModel {
                library,
                model,
                params,
                ret: match sp.ret {
                    Some(r) => Some(self.ty(db.base(r), span)?),
                    None => None,
                },
            }),
        };
        Ok(ForeignRef {
            name,
            symbol,
            args,
            ret,
            model,
        })
    }

    /// The C type of scalar subtype `st` in the usual VHPIDIRECT mapping (an integer subtype by its own
    /// range), `None` if it has none.
    fn ctype(&mut self, st: SubtypeId, span: Span) -> R<Option<CType>> {
        let db = self.db;
        let t = db.base(st);
        let v = match &db.types[t].kind {
            TypeKind::Integer => match db.static_bounds(st) {
                Some((l, _, r)) => match (l.as_int(), r.as_int()) {
                    (Some(l), Some(r)) => VhdlType::Integer {
                        low: l.min(r),
                        high: l.max(r),
                    },
                    _ => return Err(self.cx.ice(span, "integer bounds")),
                },
                // A range known only at run time: the type's own range.
                None => match self
                    .ty(t, span)
                    .map(|id| self.fb.program().types().get(id))?
                {
                    Some(Ty::Int { lo, hi }) => VhdlType::Integer {
                        low: *lo,
                        high: *hi,
                    },
                    _ => return Err(self.cx.ice(span, "integer type")),
                },
            },
            TypeKind::Enum { lits } => VhdlType::Enum {
                literals: lits.len() as u64,
            },
            TypeKind::Floating => VhdlType::Real,
            TypeKind::Physical { .. } => VhdlType::Physical,
            _ => return Ok(None),
        };
        Ok(vhdl::lower_result(Some(v)).ok().flatten())
    }
}

#[cfg(test)]
mod tests {
    use super::{vhpi_model, vhpidirect_symbol as sym};

    #[test]
    fn vhpi_forms() {
        assert_eq!(
            vhpi_model(" vhpi  lib  mod "),
            Some(("lib".into(), "mod".into()))
        );
        assert_eq!(vhpi_model("VHPI lib"), None);
        assert_eq!(vhpi_model("VHPIDIRECT lib mod"), None);
    }

    #[test]
    fn vhpidirect_forms() {
        assert_eq!(sym("VHPIDIRECT add1", "f").as_deref(), Some("add1"));
        assert_eq!(
            sym("  vhpidirect  libf.so  add1 ", "f").as_deref(),
            Some("add1")
        );
        assert_eq!(sym("VHPIDIRECT", "f").as_deref(), Some("f"));
        assert_eq!(sym("VHPI lib model", "f"), None);
        assert_eq!(sym("", "f"), None);
    }
}
