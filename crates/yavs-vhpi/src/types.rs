//! Types and subtypes as VHPI objects (#270): type declarations, subtypes, their range
//! constraints, enumeration literals, record element declarations and physical units. An owned
//! table built with the design snapshot (`design.rs`); safe Rust only.
//!
//! An object's type (`vhpiType`) is its type declaration when its subtype is the type's first
//! subtype with the bounds the declaration gives, a named subtype when the bounds are the named
//! subtype's own, else an anonymous subtype with the elaborated bounds, named like its type.

// VHPI names are matched as written in the C standard.
#![allow(non_upper_case_globals)]

use std::collections::HashMap;

use crate::design::vhpi_name;
use crate::sys::*;
use crate::value::Scalar;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TyId(pub(crate) u32);

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Ty {
    /// `vhpiEnumTypeDeclK` .. `vhpiRecordTypeDeclK` for a type, `vhpiSubtypeDeclK`.
    pub(crate) kind: vhpiClassKindT,
    /// Original spelling; an anonymous subtype has its type's name, as the reference
    /// simulator with VHPI reports it.
    pub(crate) name: String,
    pub(crate) anonymous: bool,
    /// The type declaration; `None` for one itself.
    pub(crate) base: Option<TyId>,
    pub(crate) def: Def,
    /// A scalar's range, an array's index ranges; `None`: unconstrained or not known.
    pub(crate) ranges: Option<Vec<Range>>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Def {
    /// With the units of a physical type (name, base units).
    Scalar(Scalar, Vec<(String, i64)>),
    Array {
        elem: TyId,
        dims: u32,
    },
    Record(Vec<(String, TyId)>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Bound {
    Int(i64),
    Real(f64),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Range {
    /// `vhpiIntRangeK`, `vhpiEnumRangeK`, `vhpiPhysRangeK` or `vhpiFloatRangeK`.
    pub(crate) kind: vhpiClassKindT,
    pub(crate) left: Bound,
    pub(crate) right: Bound,
    pub(crate) up: bool,
}

/// A handle's view of the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TypeObj {
    Ty(TyId),
    /// Range `i` of a type's constraints.
    Range(TyId, u32),
    /// Enumeration literal at position `i`.
    Lit(TyId, u32),
    /// Record element declaration `i`.
    Field(TyId, u32),
    /// Unit `i` of a physical type.
    Unit(TyId, u32),
}

#[derive(Default)]
pub(crate) struct Types {
    tys: Vec<Ty>,
    /// Interning: equal types are one object, so handles compare equal.
    // ponytail: keyed by the Debug text; a structural hash when type tables get large.
    by_key: HashMap<String, TyId>,
}

impl Types {
    pub(crate) fn add(&mut self, ty: Ty) -> TyId {
        let key = format!("{ty:?}");
        if let Some(&id) = self.by_key.get(&key) {
            return id;
        }
        let id = TyId(self.tys.len() as u32);
        self.tys.push(ty);
        self.by_key.insert(key, id);
        id
    }

    pub(crate) fn get(&self, t: TyId) -> &Ty {
        &self.tys[t.0 as usize]
    }

    pub(crate) fn base(&self, t: TyId) -> TyId {
        self.get(t).base.unwrap_or(t)
    }

    /// The type of sub-element `path` (record field or array element indexes) of type `t`.
    pub(crate) fn elem(&self, mut t: TyId, path: &[u64]) -> Option<TyId> {
        for &i in path {
            t = match &self.get(t).def {
                Def::Array { elem, .. } => *elem,
                Def::Record(fields) => fields.get(usize::try_from(i).ok()?)?.1,
                Def::Scalar(..) => return None,
            };
        }
        Some(t)
    }

    pub(crate) fn kind(&self, o: TypeObj) -> vhpiClassKindT {
        match o {
            TypeObj::Ty(t) => self.get(t).kind,
            TypeObj::Range(t, i) => self.range(t, i).map_or(vhpiUndefined, |r| r.kind),
            TypeObj::Lit(..) => vhpiEnumLiteralK,
            TypeObj::Field(..) => vhpiElemDeclK,
            TypeObj::Unit(..) => vhpiUnitDeclK,
        }
    }

    fn range(&self, t: TyId, i: u32) -> Option<&Range> {
        self.get(t).ranges.as_ref()?.get(i as usize)
    }

    fn scalar(&self, t: TyId) -> Option<(&Scalar, &[(String, i64)])> {
        match &self.get(t).def {
            Def::Scalar(s, units) => Some((s, units)),
            _ => None,
        }
    }

    /// Original spelling of a named object.
    pub(crate) fn name(&self, o: TypeObj) -> Option<String> {
        Some(match o {
            TypeObj::Ty(t) => self.get(t).name.clone(),
            TypeObj::Lit(t, i) => match self.scalar(t)?.0 {
                Scalar::Enum { lits, .. } => lits.get(i as usize)?.name.clone(),
                _ => return None,
            },
            TypeObj::Field(t, i) => match &self.get(t).def {
                Def::Record(f) => f.get(i as usize)?.0.clone(),
                _ => return None,
            },
            TypeObj::Unit(t, i) => self.scalar(t)?.1.get(i as usize)?.0.clone(),
            TypeObj::Range(..) => return None,
        })
    }

    pub(crate) fn str_property(&self, o: TypeObj, prop: vhpiStrPropertyT) -> Option<String> {
        let name = self.name(o)?;
        Some(match prop {
            vhpiNameP | vhpiUnitNameP => vhpi_name(&name),
            vhpiCaseNameP => name,
            // An enumeration literal's value string is the literal as written.
            vhpiStrValP if matches!(o, TypeObj::Lit(..)) => name,
            _ => return None,
        })
    }

    pub(crate) fn int_property(&self, o: TypeObj, prop: vhpiIntPropertyT) -> Option<vhpiIntT> {
        let flag = |b: bool| if b { vhpiTrue } else { vhpiFalse };
        let int = |b: Bound| match b {
            Bound::Int(v) => i32::try_from(v).ok(),
            Bound::Real(_) => None,
        };
        Some(match (o, prop) {
            (TypeObj::Ty(t), _) => {
                let ty = self.get(t);
                match (prop, &ty.def) {
                    (vhpiNumLiteralsP, Def::Scalar(Scalar::Enum { lits, .. }, _)) => {
                        i32::try_from(lits.len()).ok()?
                    }
                    (vhpiNumDimensionsP, Def::Array { dims, .. }) => i32::try_from(*dims).ok()?,
                    (vhpiNumFieldsP | DEPRECATED_vhpiNumFieldsP, Def::Record(f)) => {
                        i32::try_from(f.len()).ok()?
                    }
                    (vhpiIsUnconstrainedP, d) => {
                        flag(matches!(d, Def::Array { .. }) && ty.ranges.is_none())
                    }
                    (vhpiIsScalarP, d) => flag(matches!(d, Def::Scalar(..))),
                    (vhpiIsCompositeP, d) => flag(!matches!(d, Def::Scalar(..))),
                    (vhpiIsAnonymousP, _) => flag(ty.anonymous),
                    _ => return None,
                }
            }
            (TypeObj::Range(t, i), _) => {
                let r = self.range(t, i)?;
                match prop {
                    vhpiLeftBoundP => int(r.left)?,
                    vhpiRightBoundP => int(r.right)?,
                    vhpiIsUpP => flag(r.up),
                    // Every range handle is a constraint with bounds.
                    vhpiIsUnconstrainedP => vhpiFalse,
                    vhpiIsNullP => match (r.left, r.right) {
                        (Bound::Int(l), Bound::Int(h)) => flag(if r.up { l > h } else { l < h }),
                        (Bound::Real(l), Bound::Real(h)) => flag(if r.up { l > h } else { l < h }),
                        _ => return None,
                    },
                    _ => return None,
                }
            }
            (TypeObj::Lit(_, i) | TypeObj::Field(_, i), vhpiPositionP) => i32::try_from(i).ok()?,
            _ => return None,
        })
    }

    pub(crate) fn real_property(&self, o: TypeObj, prop: vhpiRealPropertyT) -> Option<f64> {
        let TypeObj::Range(t, i) = o else {
            return None;
        };
        let r = self.range(t, i)?;
        match (prop, r.left, r.right) {
            (vhpiFloatLeftBoundP, Bound::Real(v), _)
            | (vhpiFloatRightBoundP, _, Bound::Real(v)) => Some(v),
            _ => None,
        }
    }

    /// Physical bounds and unit positions, in base units.
    pub(crate) fn phys_property(&self, o: TypeObj, prop: vhpiPhysPropertyT) -> Option<i64> {
        match (o, prop) {
            (TypeObj::Range(t, i), vhpiPhysLeftBoundP | vhpiPhysRightBoundP) => {
                let r = self.range(t, i)?;
                if r.kind != vhpiPhysRangeK {
                    return None;
                }
                match if prop == vhpiPhysLeftBoundP {
                    r.left
                } else {
                    r.right
                } {
                    Bound::Int(v) => Some(v),
                    Bound::Real(_) => None,
                }
            }
            (TypeObj::Unit(t, i), vhpiPhysPositionP) => Some(self.scalar(t)?.1.get(i as usize)?.1),
            _ => None,
        }
    }

    /// `vhpi_handle(rel, o)` within the table; `None`: no such object.
    pub(crate) fn one_to_one(&self, rel: vhpiOneToOneT, o: TypeObj) -> Option<TypeObj> {
        let t = match o {
            TypeObj::Ty(t) => t,
            TypeObj::Field(t, i) => match &self.get(t).def {
                Def::Record(f) => {
                    let ft = f.get(i as usize)?.1;
                    return match rel {
                        vhpiType | DEPRECATED_vhpiSubtype => Some(TypeObj::Ty(ft)),
                        vhpiBaseType => Some(TypeObj::Ty(self.base(ft))),
                        _ => None,
                    };
                }
                _ => return None,
            },
            _ => return None,
        };
        match (rel, &self.get(t).def) {
            (vhpiBaseType, _) => Some(TypeObj::Ty(self.base(t))),
            (vhpiElemType | DEPRECATED_vhpiElemSubtype, Def::Array { elem, .. }) => {
                Some(TypeObj::Ty(*elem))
            }
            _ => None,
        }
    }

    /// `vhpi_iterator(rel, type t)`: the objects, `None` when the relation does not apply.
    pub(crate) fn iterate(&self, rel: vhpiOneToManyT, t: TyId) -> Option<Vec<TypeObj>> {
        let ty = self.get(t);
        let n =
            |len: usize, f: fn(TyId, u32) -> TypeObj| (0..len as u32).map(|i| f(t, i)).collect();
        Some(match (rel, &ty.def) {
            (vhpiConstraints, _) => n(ty.ranges.as_ref().map_or(0, Vec::len), TypeObj::Range),
            (vhpiEnumLiterals, Def::Scalar(Scalar::Enum { lits, .. }, _)) => {
                n(lits.len(), TypeObj::Lit)
            }
            (vhpiRecordElems, Def::Record(f)) => n(f.len(), TypeObj::Field),
            (vhpiUnitDecls, Def::Scalar(Scalar::Phys { .. }, units)) => {
                n(units.len(), TypeObj::Unit)
            }
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interned_types_and_their_objects() {
        let mut ts = Types::default();
        let int = Ty {
            kind: vhpiIntTypeDeclK,
            name: "integer".into(),
            anonymous: false,
            base: None,
            def: Def::Scalar(Scalar::Int { narrow: false }, Vec::new()),
            ranges: Some(vec![Range {
                kind: vhpiIntRangeK,
                left: Bound::Int(-5),
                right: Bound::Int(5),
                up: true,
            }]),
        };
        let i = ts.add(int.clone());
        assert_eq!(ts.add(int), i);
        let arr = ts.add(Ty {
            kind: vhpiSubtypeDeclK,
            name: "integer_vector".into(),
            anonymous: true,
            base: None,
            def: Def::Array { elem: i, dims: 1 },
            ranges: None,
        });
        assert_eq!(
            ts.int_property(TypeObj::Ty(arr), vhpiIsUnconstrainedP),
            Some(1)
        );
        assert_eq!(ts.int_property(TypeObj::Ty(arr), vhpiIsAnonymousP), Some(1));
        assert_eq!(
            ts.one_to_one(vhpiElemType, TypeObj::Ty(arr)),
            Some(TypeObj::Ty(i))
        );
        assert_eq!(ts.elem(arr, &[3]), Some(i));
        let rs = ts.iterate(vhpiConstraints, i).unwrap();
        assert_eq!(rs, [TypeObj::Range(i, 0)]);
        assert_eq!(ts.int_property(rs[0], vhpiLeftBoundP), Some(-5));
        assert_eq!(ts.int_property(rs[0], vhpiIsUpP), Some(1));
        assert_eq!(ts.kind(rs[0]), vhpiIntRangeK);
        assert_eq!(
            ts.str_property(TypeObj::Ty(i), vhpiNameP).as_deref(),
            Some("INTEGER")
        );
        assert_eq!(ts.iterate(vhpiRecordElems, i), None);
    }
}
