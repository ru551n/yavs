//! Types, subtypes and constraints (sem.md section 2, plan rule 1).
//!
//! There is exactly one subtype representation and every constraint in it is
//! data: range bounds are HIR expressions, never numbers. A folded value on an
//! expression is a cache, not a second representation.

use yavs_syntax::ast::Ident;

use crate::generic::GenericClass;
use crate::hir::AttrPrefix;
use crate::ids::{DeclId, ExprId, Id, ScopeId, SubtypeId, TypeId};
use yavs_diag::Span;

pub use yavs_syntax::ast::Direction;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct TypeDef {
    pub decl: DeclId,
    /// For an anonymous base type: the declared name, with `anonymous = true`.
    pub name: Ident,
    /// Diagnostics print "the base type of `T`".
    pub anonymous: bool,
    pub kind: TypeKind,
    /// `T` in `type T is ...`.
    pub first_subtype: SubtypeId,
    /// The subtype covering the whole base range (`T'base`): for integer,
    /// floating and physical types the implementation-defined base range (Q2);
    /// otherwise the same as `first_subtype`.
    pub base_subtype: SubtypeId,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub enum TypeKind {
    UniversalInteger,
    UniversalReal,
    /// `EnumLit` decls in position order.
    Enum {
        lits: Box<[DeclId]>,
    },
    /// Ranges live in subtypes.
    Integer,
    Floating,
    Physical {
        units: Box<[PhysUnit]>,
    },
    /// Index subtypes (`natural range <>`) and element subtype.
    Array {
        indexes: Box<[SubtypeId]>,
        elem: SubtypeId,
    },
    Record {
        elems: Box<[RecordElem]>,
    },
    Access {
        designated: SubtypeId,
    },
    File {
        elem: SubtypeId,
    },
    Protected {
        decl_scope: ScopeId,
        /// The protected body's region (its parent is `decl_scope`), once paired.
        body: Option<ScopeId>,
        generics: Option<Box<[DeclId]>>,
    },
    Incomplete {
        completion: Option<TypeId>,
    },
    /// Opaque inside a generic unit (2.9).
    Generic {
        class: GenericClass,
    },
    /// Poison (sem.md 8.2): compatible with everything, never diagnosed again.
    Error,
}

/// A unit of a physical type; `scale` is in base units.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct PhysUnit {
    pub decl: DeclId,
    pub scale: i64,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct RecordElem {
    pub name: Ident,
    pub subtype: SubtypeId,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct SubtypeDef {
    pub base: TypeId,
    /// `None`: an anonymous subtype indication.
    pub name: Option<Ident>,
    /// The type mark it was derived from (diagnostics, `'base`). Consumers never
    /// walk it to find constraints: `constraint` is already merged.
    pub parent: Option<SubtypeId>,
    /// The effective constraint, the parent's merged in.
    pub constraint: Constraint,
    pub resolution: Option<Resolution>,
    pub span: Span,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum Constraint {
    /// Scalars: the base range; composites: nothing added at this level.
    None,
    Range(RangeC),
    /// `dims: None` is `open` or `<>`.
    Array {
        dims: Option<Box<[RangeC]>>,
        elem: Box<Constraint>,
    },
    /// Only the constrained elements, by element index.
    Record {
        elems: Box<[(u32, Constraint)]>,
    },
    /// `x'subtype`, `x'element`: taken from the object at runtime.
    FromObject {
        expr: ExprId,
        elem_depth: u8,
    },
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum RangeC {
    Explicit {
        left: ExprId,
        dir: Direction,
        right: ExprId,
    },
    /// `a'range`, `t'reverse_range(2)`.
    Attr {
        prefix: AttrPrefix,
        reverse: bool,
        dim: u32,
    },
}

/// Resolution indication (6.3).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum Resolution {
    /// `resolved std_ulogic`
    Function(DeclId),
    /// `(resolved) std_ulogic_vector` (2008)
    Element(Box<Resolution>),
    /// `(a resolved_a, b resolved_b) rec_t`
    Record(Box<[(u32, Resolution)]>),
}

/// Where an object's bounds come from (2.2), stored on the object so M2 never guesses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum BoundsFrom {
    /// Fully constrained subtype indication.
    Subtype,
    /// Constants (6.4.2.2) and allocators `new T'(x)`.
    Initializer,
    /// Interface objects with not fully constrained subtypes.
    Actual,
    /// Object alias without subtype indication.
    Aliased,
}

/// Computed, never stored (5.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Constrainedness {
    Full,
    Partial,
    Unconstrained,
}

/// Coarse classification used by implicit operations, conversions and generic classes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeClass {
    Enum,
    Integer,
    Floating,
    Physical,
    Array,
    Record,
    Access,
    File,
    Protected,
    Incomplete,
    Generic,
    Error,
}

impl TypeKind {
    pub fn class(&self) -> TypeClass {
        match self {
            TypeKind::UniversalInteger | TypeKind::Integer => TypeClass::Integer,
            TypeKind::UniversalReal | TypeKind::Floating => TypeClass::Floating,
            TypeKind::Enum { .. } => TypeClass::Enum,
            TypeKind::Physical { .. } => TypeClass::Physical,
            TypeKind::Array { .. } => TypeClass::Array,
            TypeKind::Record { .. } => TypeClass::Record,
            TypeKind::Access { .. } => TypeClass::Access,
            TypeKind::File { .. } => TypeClass::File,
            TypeKind::Protected { .. } => TypeClass::Protected,
            TypeKind::Incomplete { .. } => TypeClass::Incomplete,
            TypeKind::Generic { .. } => TypeClass::Generic,
            TypeKind::Error => TypeClass::Error,
        }
    }

    pub fn is_scalar(&self) -> bool {
        matches!(
            self.class(),
            TypeClass::Enum | TypeClass::Integer | TypeClass::Floating | TypeClass::Physical
        )
    }

    pub fn is_discrete(&self) -> bool {
        matches!(self.class(), TypeClass::Enum | TypeClass::Integer)
    }
}

// ---------------------------------------------------------------------------
// Analysis of type and subtype declarations and subtype indications.

use yavs_diag::Diagnostic;
use yavs_syntax::Symbol;
use yavs_syntax::ast;

use crate::LangRev;
use crate::db::{Db, Reported};
use crate::decl::{Decl, DeclKind, Designator};
use crate::diag;
use crate::expr::Expected;
use crate::hir::{Expr, ExprKind};
use crate::stat::{ConstValue, Staticness};

impl Db {
    pub(crate) fn new_decl(
        &mut self,
        scope: ScopeId,
        name: Designator,
        kind: DeclKind,
        span: Span,
    ) -> DeclId {
        self.decls.push(Decl {
            name,
            kind,
            scope,
            span,
            implicit: false,
            since: LangRev::Vhdl1993,
        })
    }

    /// A constant expression of subtype `st` (for implementation-defined bounds).
    pub(crate) fn const_expr(&mut self, v: ConstValue, st: SubtypeId, span: Span) -> ExprId {
        let c = self.add_const(v);
        self.add_expr(Expr {
            kind: ExprKind::Const(c),
            subtype: st,
            span,
            stat: Staticness::Locally,
            value: Some(c),
        })
    }

    /// A new type with a first subtype; `base_subtype` is the first subtype too.
    pub(crate) fn new_type(
        &mut self,
        decl: DeclId,
        name: Ident,
        anonymous: bool,
        kind: TypeKind,
        constraint: Constraint,
        span: Span,
    ) -> TypeId {
        let ty = self.types.next_id();
        let st = self.subtypes.push(SubtypeDef {
            base: ty,
            name: Some(name),
            parent: None,
            constraint,
            resolution: None,
            span,
        });
        self.types.push(TypeDef {
            decl,
            name,
            anonymous,
            kind,
            first_subtype: st,
            base_subtype: st,
        })
    }

    /// `type T is ...;` (5.x).
    pub(crate) fn type_decl(&mut self, scope: ScopeId, td: &ast::TypeDecl) -> Result<(), Reported> {
        let name = td.name;
        // Completion of an incomplete type in the same region reuses its entity.
        let incomplete = self.scopes[scope].names.get(&name.key).and_then(|ds| {
            ds.iter().copied().find(|&d| {
                matches!(self.decls[d].kind, DeclKind::Type(t) if matches!(self.types[t].kind, TypeKind::Incomplete { completion: None }))
            })
        });
        let Some(def) = &td.def else {
            let decl = self.new_decl(scope, Designator::Ident(name), DeclKind::Error, td.span);
            let ty = self.new_type(
                decl,
                name,
                false,
                TypeKind::Incomplete { completion: None },
                Constraint::None,
                td.span,
            );
            self.decls[decl].kind = DeclKind::Type(ty);
            self.declare(scope, decl);
            return Ok(());
        };
        if let ast::TypeDef::ProtectedBody(b) = def {
            return self.protected_body(scope, name, b, td.span);
        }
        let first_subtype = self.subtypes.next_id();
        let decl = match incomplete {
            Some(d) => d,
            None => self.new_decl(scope, Designator::Ident(name), DeclKind::Error, td.span),
        };
        let ty = match def {
            ast::TypeDef::Enum(lits) => self.enum_type(scope, decl, name, lits, td.span),
            ast::TypeDef::Range(r) => self.range_type(scope, decl, name, r, None, td.span)?,
            ast::TypeDef::Physical(p) => {
                self.range_type(scope, decl, name, &p.range, Some(p), td.span)?
            }
            ast::TypeDef::Array { indexes, element } => {
                self.array_type(scope, decl, name, indexes, element, td.span)?
            }
            ast::TypeDef::Access(si) => {
                self.incomplete_ok = true;
                let designated = self.subtype_indication(scope, si);
                self.incomplete_ok = false;
                let designated = designated?;
                self.check_designated_type(designated, si.span)?;
                self.new_type(
                    decl,
                    name,
                    false,
                    TypeKind::Access { designated },
                    Constraint::None,
                    td.span,
                )
            }
            ast::TypeDef::File(n) => {
                let elem = self.type_mark(scope, n)?;
                // 5.5.1: not a multidimensional array (#813; sim1 and sim2 agree in 2008).
                if let TypeKind::Array { indexes, .. } = &self.types[self.base(elem)].kind
                    && indexes.len() > 1
                {
                    return Err(self.error(
                        Diagnostic::error(
                            diag::BAD_FILE_ELEMENT,
                            format!(
                                "a file cannot contain multidimensional array type `{}`",
                                self.type_name(self.base(elem))
                            ),
                        )
                        .with_primary(n.span, "not a valid file element type"),
                    ));
                }
                if let Some((what, bad)) = self.bad_elem(self.base(elem), true) {
                    return Err(self.error(
                        Diagnostic::error(
                            diag::BAD_FILE_ELEMENT,
                            format!(
                                "a file cannot contain {what} type `{}`",
                                self.type_name(bad)
                            ),
                        )
                        .with_primary(n.span, "not a valid file element type"),
                    ));
                }
                self.new_type(
                    decl,
                    name,
                    false,
                    TypeKind::File { elem },
                    Constraint::None,
                    td.span,
                )
            }
            ast::TypeDef::Record { elements, .. } => {
                self.record_type(scope, decl, name, elements, td.span)?
            }
            ast::TypeDef::Protected(pt) => {
                return self.protected_decl(scope, decl, name, pt, td.span);
            }
            ast::TypeDef::ProtectedInstance {
                protected,
                generic_map,
            } => {
                let map = generic_map.as_deref().unwrap_or(&[]);
                return self.protected_instance(scope, decl, name, protected, map, td.span);
            }
            ast::TypeDef::ProtectedBody(_) => unreachable!("handled above"),
        };
        if let Some(d) = incomplete {
            // Move the full definition into the incomplete type's id, so earlier references stay valid.
            let DeclKind::Type(old) = self.decls[d].kind else {
                unreachable!("checked above")
            };
            let full = std::mem::replace(
                &mut self.types[ty].kind,
                TypeKind::Incomplete {
                    completion: Some(old),
                },
            );
            self.types[old].kind = full;
            self.types[old].anonymous = self.types[ty].anonymous;
            // Every subtype of the full definition belongs to `old`.
            for i in first_subtype.index()..self.subtypes.len() {
                let s = SubtypeId::from_index(i);
                if self.subtypes[s].base == ty {
                    self.subtypes[s].base = old;
                }
            }
            // Access types declared before the completion designate the incomplete
            // type's first subtype: it takes the full first subtype's constraint.
            let (old_fs, fs) = (self.types[old].first_subtype, self.types[ty].first_subtype);
            let c = self.subtypes[fs].constraint.clone();
            let parent = self.subtypes[fs].parent;
            self.subtypes[old_fs].constraint = c;
            self.subtypes[old_fs].parent = parent;
            let bs = self.types[ty].base_subtype;
            self.types[old].base_subtype = if bs == fs { old_fs } else { bs };
            let lits: Vec<DeclId> = match &self.types[old].kind {
                TypeKind::Enum { lits } => lits.to_vec(),
                TypeKind::Physical { units } => units.iter().map(|u| u.decl).collect(),
                _ => Vec::new(),
            };
            for l in lits {
                match &mut self.decls[l].kind {
                    DeclKind::EnumLit { ty: lt, .. } | DeclKind::PhysUnit { ty: lt, .. } => {
                        *lt = old
                    }
                    _ => {}
                }
                self.declare(scope, l);
            }
            self.register_well_known_type(scope, name, old);
            self.declare_implicit_ops(old, scope);
            return Ok(());
        }
        self.decls[decl].kind = DeclKind::Type(ty);
        self.declare(scope, decl);
        if let TypeKind::Physical { units } = &self.types[ty].kind {
            for u in units.clone().iter() {
                self.declare(scope, u.decl);
            }
        }
        if let TypeKind::Enum { lits } = &self.types[ty].kind {
            for &l in lits.clone().iter() {
                self.declare(scope, l);
            }
        }
        self.register_well_known_type(scope, name, ty);
        self.declare_implicit_ops(ty, scope);
        Ok(())
    }

    /// `type T is protected ... end protected;` (5.6.2). `T` is visible in its own
    /// method profiles.
    fn protected_decl(
        &mut self,
        scope: ScopeId,
        decl: DeclId,
        name: Ident,
        pt: &ast::ProtectedDecl,
        span: Span,
    ) -> Result<(), Reported> {
        let pscope = self.add_scope(crate::scope::ScopeKind::Protected, Some(scope), Some(decl));
        let ty = self.new_type(
            decl,
            name,
            false,
            TypeKind::Protected {
                decl_scope: pscope,
                body: None,
                generics: None,
            },
            Constraint::None,
            span,
        );
        self.decls[decl].kind = DeclKind::Type(ty);
        self.declare(scope, decl);
        self.check_protected_decl_items(pt)?;
        if let Some(g) = &pt.generics {
            // 2019 5.6.2: the generics are opaque here; a variable instantiates them.
            if let Some(m) = pt.generic_map.as_ref().and_then(|m| m.first()) {
                return Err(self.unsupported(
                    m.span,
                    "a generic map in a protected type header",
                    "S7",
                ));
            }
            let list = self.generic_list(pscope, g)?;
            if let TypeKind::Protected { generics, .. } = &mut self.types[ty].kind {
                *generics = Some(list);
            }
        }
        self.decls_in(pscope, &pt.decls);
        let aliases: Vec<DeclId> = self.scopes[pscope]
            .order
            .iter()
            .copied()
            .filter(|&d| matches!(self.decls[d].kind, DeclKind::Alias(_)))
            .collect();
        for a in aliases {
            let _ = self.check_protected_alias(a);
        }
        Ok(())
    }

    /// `type T is protected body ... end protected body;` (5.6.3).
    fn protected_body(
        &mut self,
        scope: ScopeId,
        name: Ident,
        b: &ast::ProtectedBody,
        span: Span,
    ) -> Result<(), Reported> {
        let ty_decl = match self.lookup(scope, name.key) {
            crate::scope::Visible::One(d) => d,
            crate::scope::Visible::None => return Err(self.undeclared(scope, name)),
            _ => {
                return Err(self.error(
                    Diagnostic::error(
                        diag::NOT_A_TYPE,
                        format!("`{}` is not a protected type", name.text),
                    )
                    .with_primary(name.span, "expected a protected type declaration"),
                ));
            }
        };
        let bscope = self.add_scope(
            crate::scope::ScopeKind::ProtectedBody,
            Some(scope),
            Some(ty_decl),
        );
        let t = self.pair_protected_body(scope, ty_decl, name, bscope)?;
        self.protected_body_generics(t, bscope);
        self.check_protected_body_items(b)?;
        self.decls_in(bscope, &b.decls);
        self.check_protected_complete(t, span)
    }

    /// `type R is record ... end record;` (5.3.3). Element names are reachable only by selection.
    fn record_type(
        &mut self,
        scope: ScopeId,
        decl: DeclId,
        name: Ident,
        elements: &[ast::ElementDecl],
        span: Span,
    ) -> Result<TypeId, Reported> {
        let ty = self.types.next_id();
        let mut elems: Vec<RecordElem> = Vec::new();
        let mut failed = false;
        for e in elements {
            let st = match self.subtype_indication(scope, &e.subtype) {
                Ok(st) => st,
                Err(_) => {
                    failed = true;
                    self.poison.subtype
                }
            };
            if self.check_elem_constrained_93(st, e.subtype.span).is_err()
                || self.check_elem_not_file(st, e.subtype.span).is_err()
            {
                failed = true;
            }
            // 2019 allows record elements of protected types (LCS-2016-014).
            if self.file_rev() < crate::LangRev::Vhdl2019
                && self
                    .check_not_protected("record element", st, e.subtype.span)
                    .is_err()
            {
                failed = true;
            }
            for &n in &e.names {
                if let Some(prev) = elems.iter().find(|x| x.name.key == n.key) {
                    let prev = prev.name.span;
                    self.report(
                        Diagnostic::error(
                            diag::REDECLARED,
                            format!("record element `{}` is declared twice", n.text),
                        )
                        .with_primary(n.span, "redeclared here")
                        .with_secondary(prev, "first declared here"),
                    );
                    failed = true;
                    continue;
                }
                elems.push(RecordElem {
                    name: n,
                    subtype: st,
                });
            }
        }
        // 2019 5.3.1: if some element is protected, all must be.
        if !failed
            && elems
                .iter()
                .any(|e| self.type_contains_protected(self.base(e.subtype)))
        {
            let bad: Vec<_> = elems
                .iter()
                .filter(|e| !self.all_protected(self.base(e.subtype)))
                .map(|e| e.name)
                .collect();
            for name in bad {
                self.report(
                    Diagnostic::error(
                        diag::PROTECTED_MISUSE,
                        format!(
                            "record element `{}` is not of a protected type, but another element is",
                            name.text
                        ),
                    )
                    .with_primary(name.span, "not a protected type")
                    .with_note("LRM 5.3.1 (2019): a record with a protected element can only have protected elements")
                    .with_help("move the other elements into a protected type"),
                );
                failed = true;
            }
        }
        if failed {
            return Err(Reported);
        }
        let got = self.new_type(
            decl,
            name,
            false,
            TypeKind::Record {
                elems: elems.into(),
            },
            Constraint::None,
            span,
        );
        debug_assert_eq!(got, ty);
        Ok(got)
    }

    /// The index of record element `key` of record type `t`.
    pub fn record_elem(&self, t: TypeId, key: Symbol) -> Option<u32> {
        match &self.types[t].kind {
            TypeKind::Record { elems } => elems
                .iter()
                .position(|e| e.name.key == key)
                .map(|i| i as u32),
            _ => None,
        }
    }

    fn enum_type(
        &mut self,
        scope: ScopeId,
        decl: DeclId,
        name: Ident,
        lits: &[ast::EnumLit],
        span: Span,
    ) -> TypeId {
        let ty = self.types.next_id();
        let mut ids = Vec::with_capacity(lits.len());
        for (pos, l) in lits.iter().enumerate() {
            let (n, sp) = match l {
                ast::EnumLit::Ident(i) => (Designator::Ident(*i), i.span),
                ast::EnumLit::Char(c) => (Designator::Char(c.value), c.span),
            };
            if ids
                .iter()
                .any(|&d: &DeclId| self.decls[d].name.key() == n.key())
            {
                self.report(
                    Diagnostic::error(
                        diag::REDECLARED,
                        format!("enumeration literal {n} appears twice"),
                    )
                    .with_primary(sp, "repeated here"),
                );
            }
            ids.push(self.new_decl(
                scope,
                n,
                DeclKind::EnumLit {
                    ty,
                    pos: pos as u32,
                },
                sp,
            ));
        }
        self.new_type(
            decl,
            name,
            false,
            TypeKind::Enum { lits: ids.into() },
            Constraint::None,
            span,
        )
    }

    /// Integer, floating point and physical type definitions (5.2.3, 5.2.4, 5.2.5).
    fn range_type(
        &mut self,
        scope: ScopeId,
        decl: DeclId,
        name: Ident,
        r: &ast::Range,
        phys: Option<&ast::PhysicalTypeDef>,
        span: Span,
    ) -> Result<TypeId, Reported> {
        let (l, rr, dir, lv, rv, bounds_span) = match r {
            ast::Range::Simple { left, dir, right } => {
                let before = self.error_count();
                let l = self.static_expr(scope, left, Expected::Any);
                let rr = self.static_expr(scope, right, Expected::Any);
                let (lv, rv) = match (self.value(l).cloned(), self.value(rr).cloned()) {
                    (Some(a), Some(b)) => (a, b),
                    // Already reported (an overflow, a division by zero): no follow-on `E1410`.
                    _ if self.error_count() > before => return Err(Reported),
                    _ => {
                        for (e, ast) in [(l, left), (rr, right)] {
                            if self.value(e).is_none() && !self.is_poison_expr(e) {
                                self.report(
                                    Diagnostic::error(
                                        diag::NOT_STATIC,
                                        "the bound of a type definition must be locally static",
                                    )
                                    .with_primary(ast.span, "not locally static"),
                                );
                            }
                        }
                        return Err(Reported);
                    }
                };
                (l, rr, *dir, lv, rv, left.span.to(right.span))
            }
            // `range t'range(2)`: the bounds are the folded range of an array dimension or of
            // a scalar subtype, as locally static integers (3.1.2, 5.2.1).
            ast::Range::Name(n) => {
                let Some(res) = self.range_attr(scope, n) else {
                    // 5.2.1: a range is `l to r` or a range attribute, never a type mark.
                    return Err(self.error(
                        Diagnostic::error(
                            diag::BAD_TYPE_DECL,
                            "the range of a type definition must be `left to right` or a range attribute",
                        )
                        .with_primary(n.span, "not a range")
                        .with_help("write the bounds, e.g. `range t'low to t'high`, or `range t'range`"),
                    ));
                };
                let (rc, t) = res?;
                let Some(sr) = crate::expr::aggregate::range_value(self, &rc) else {
                    return Err(self.error(
                        Diagnostic::error(
                            diag::NOT_STATIC,
                            "the bound of a type definition must be locally static",
                        )
                        .with_primary(n.span, "not locally static"),
                    ));
                };
                if !matches!(
                    self.types[t].kind,
                    TypeKind::Integer | TypeKind::UniversalInteger
                ) || phys.is_some()
                {
                    return Err(self.error(
                        Diagnostic::error(
                            diag::BAD_TYPE_DECL,
                            "the range of an integer type definition must be of an integer type",
                        )
                        .with_primary(n.span, "not an integer range"),
                    ));
                }
                let uint = self.well_known.universal_integer.unwrap_or(self.poison.ty);
                let mk = |db: &mut Db, v: i64| {
                    crate::expr::literal::const_expr(db, n.span, uint, Some(ConstValue::Int(v)))
                };
                let (l, rr) = (mk(self, sr.left), mk(self, sr.right));
                (
                    l,
                    rr,
                    sr.dir,
                    ConstValue::Int(sr.left),
                    ConstValue::Int(sr.right),
                    n.span,
                )
            }
        };
        let kind = match (&lv, &rv, phys) {
            (ConstValue::Int(_), ConstValue::Int(_), None) => TypeKind::Integer,
            (ConstValue::Int(_), ConstValue::Int(_), Some(_)) => TypeKind::Physical {
                units: Box::new([]),
            },
            (ConstValue::Real(_), ConstValue::Real(_), None) => TypeKind::Floating,
            _ => {
                return Err(self.error(
                    Diagnostic::error(
                        diag::BAD_TYPE_DECL,
                        "the bounds must both be integer or both be floating point",
                    )
                    .with_primary(bounds_span, "bounds of different or unsuitable types")
                    .with_help(if phys.is_some() {
                        "physical type bounds are integers"
                    } else {
                        "write both bounds as integer literals or both as real literals"
                    }),
                ));
            }
        };
        // Anonymous base type (Q2: its range is the declared range) and the named first subtype.
        let ty = self.types.next_id();
        let base_st = self.subtypes.next_id();
        let conv = |db: &mut Db, e: ExprId, v: ConstValue| {
            let c = db.add_const(v);
            let sp = db.exprs[e].span;
            // 5.2.3: a bound may be of any integer (floating) type; only a universal one converts implicitly.
            let universal = matches!(
                db.types[db.base(db.exprs[e].subtype)].kind,
                TypeKind::UniversalInteger | TypeKind::UniversalReal
            );
            db.add_expr(Expr {
                kind: if universal {
                    ExprKind::ImplicitConv { operand: e }
                } else {
                    ExprKind::Conv {
                        to: base_st,
                        operand: e,
                    }
                },
                subtype: base_st,
                span: sp,
                stat: Staticness::Locally,
                value: Some(c),
            })
        };
        let lc = conv(self, l, lv.clone());
        let rc = conv(self, rr, rv.clone());
        let range = Constraint::Range(RangeC::Explicit {
            left: lc,
            dir,
            right: rc,
        });
        self.subtypes.push(SubtypeDef {
            base: ty,
            name: Some(name),
            parent: None,
            constraint: range.clone(),
            resolution: None,
            span,
        });
        let first = self.subtypes.push(SubtypeDef {
            base: ty,
            name: Some(name),
            parent: Some(base_st),
            constraint: range,
            resolution: None,
            span,
        });
        self.types.push(TypeDef {
            decl,
            name,
            anonymous: true,
            kind,
            first_subtype: first,
            base_subtype: base_st,
        });
        if let Some(p) = phys {
            let units = self.physical_units(scope, ty, p)?;
            self.types[ty].kind = TypeKind::Physical {
                units: units.into(),
            };
        }
        Ok(ty)
    }

    fn physical_units(
        &mut self,
        scope: ScopeId,
        ty: TypeId,
        p: &ast::PhysicalTypeDef,
    ) -> Result<Vec<PhysUnit>, Reported> {
        let mut units = vec![PhysUnit {
            decl: self.new_decl(
                scope,
                Designator::Ident(p.primary),
                DeclKind::PhysUnit { ty, index: 0 },
                p.primary.span,
            ),
            scale: 1,
        }];
        for (i, (name, e)) in p.secondary.iter().enumerate() {
            let find = |db: &Db, units: &[PhysUnit], u: &Ident| {
                units
                    .iter()
                    .find(|x| db.decls[x.decl].name.key() == u.key)
                    .map(|x| x.scale)
            };
            let (value, unit) = match &e.kind {
                ast::ExprKind::Literal(ast::Literal {
                    kind: ast::LiteralKind::Physical { value, unit },
                    ..
                }) => (Some(*value), *unit),
                ast::ExprKind::Name(ast::Name {
                    kind: ast::NameKind::Simple(u),
                    ..
                }) => (None, *u),
                _ => {
                    return Err(self.error(
                        Diagnostic::error(
                            diag::BAD_TYPE_DECL,
                            "a secondary unit must be defined by a physical literal",
                        )
                        .with_primary(e.span, "expected `<integer> <unit>`"),
                    ));
                }
            };
            let Some(scale) = find(self, &units, &unit) else {
                return Err(self.error(
                    Diagnostic::error(
                        diag::NOT_DECLARED,
                        format!("`{}` is not a unit of this type", unit.text),
                    )
                    .with_primary(unit.span, "not declared before this unit"),
                ));
            };
            let scale = match value {
                None => scale,
                Some(v) => match crate::expr::literal::abstract_value(self, v, e.span)? {
                    crate::expr::literal::Abs::Int(n) => n.checked_mul(scale).ok_or_else(|| {
                        self.error(
                            Diagnostic::error(diag::OVERFLOW, "the unit's scale overflows 64 bits")
                                .with_primary(name.span, "this unit")
                                .with_secondary(e.span, "too large in base units"),
                        )
                    })?,
                    crate::expr::literal::Abs::Real(_) => {
                        return Err(self.error(
                            Diagnostic::error(
                                diag::BAD_TYPE_DECL,
                                "a secondary unit must be an integer multiple of another unit",
                            )
                            .with_primary(e.span, "real value"),
                        ));
                    }
                },
            };
            let decl = self.new_decl(
                scope,
                Designator::Ident(*name),
                DeclKind::PhysUnit {
                    ty,
                    index: i as u32 + 1,
                },
                name.span,
            );
            units.push(PhysUnit { decl, scale });
        }
        Ok(units)
    }

    fn array_type(
        &mut self,
        scope: ScopeId,
        decl: DeclId,
        name: Ident,
        indexes: &ast::ArrayIndexes,
        element: &ast::SubtypeIndication,
        span: Span,
    ) -> Result<TypeId, Reported> {
        match indexes {
            ast::ArrayIndexes::Unconstrained(marks) => {
                let mut idx = Vec::new();
                for m in marks {
                    idx.push(self.discrete_type_mark(scope, m)?);
                }
                let elem = self.subtype_indication(scope, element)?;
                self.check_elem_constrained_93(elem, element.span)?;
                self.check_elem_not_file(elem, element.span)?;
                let n = idx.len();
                let _ = n;
                Ok(self.new_type(
                    decl,
                    name,
                    false,
                    TypeKind::Array {
                        indexes: idx.into(),
                        elem,
                    },
                    Constraint::Array {
                        dims: None,
                        elem: Box::new(Constraint::None),
                    },
                    span,
                ))
            }
            ast::ArrayIndexes::Constrained(ranges) => {
                let mut dims = Vec::new();
                let mut idx = Vec::new();
                for r in ranges {
                    let (rc, ix) = self.index_constraint(scope, r)?;
                    dims.push(rc);
                    idx.push(ix);
                }
                let elem = self.subtype_indication(scope, element)?;
                self.check_elem_constrained_93(elem, element.span)?;
                self.check_elem_not_file(elem, element.span)?;
                let ty = self.types.next_id();
                let base_st = self.subtypes.push(SubtypeDef {
                    base: ty,
                    name: Some(name),
                    parent: None,
                    constraint: Constraint::Array {
                        dims: None,
                        elem: Box::new(Constraint::None),
                    },
                    resolution: None,
                    span,
                });
                let first = self.subtypes.push(SubtypeDef {
                    base: ty,
                    name: Some(name),
                    parent: Some(base_st),
                    constraint: Constraint::Array {
                        dims: Some(dims.into()),
                        elem: Box::new(Constraint::None),
                    },
                    resolution: None,
                    span,
                });
                Ok(self.types.push(TypeDef {
                    decl,
                    name,
                    anonymous: true,
                    kind: TypeKind::Array {
                        indexes: idx.into(),
                        elem,
                    },
                    first_subtype: first,
                    base_subtype: base_st,
                }))
            }
            ast::ArrayIndexes::Anonymous(_) => {
                Err(self.unsupported(span, "anonymous generic index types", "S7"))
            }
        }
    }

    /// `E1007`: in a VHDL-1993 file the element subtype of an array type and the subtype of a
    /// record element are constrained (1993 3.2.1, 3.2.1.1; 2008 relaxed both, #814).
    /// 5.3.1: an element of a composite type is not of a file type (#813).
    fn check_elem_not_file(&mut self, st: SubtypeId, span: Span) -> Result<(), Reported> {
        let t = self.base(st);
        if !matches!(self.types[t].kind, TypeKind::File { .. }) {
            return Ok(());
        }
        Err(self.error(
            Diagnostic::error(
                diag::BAD_TYPE_DECL,
                format!(
                    "an element of a composite type cannot be of file type `{}`",
                    self.type_name(t)
                ),
            )
            .with_primary(span, "file type")
            .with_note("LRM 5.3.1: the elements of an array or record type are not of a file type"),
        ))
    }

    fn check_elem_constrained_93(&mut self, st: SubtypeId, span: Span) -> Result<(), Reported> {
        if self.file_rev() != crate::LangRev::Vhdl1993
            || self.constrainedness(st) == Constrainedness::Full
            || matches!(self.types[self.base(st)].kind, TypeKind::Error)
        {
            return Ok(());
        }
        Err(self.error(
            Diagnostic::error(
                diag::NOT_IN_REVISION,
                "an element of an unconstrained array subtype is not allowed in VHDL-1993",
            )
            .with_primary(span, "not constrained")
            .with_note("LRM 1993 3.2.1: the element subtype indication of an array type and the subtype of a record element denote constrained subtypes (VHDL-2008 allows unconstrained ones)"),
        ))
    }

    /// One range of an array type's index constraint and its index subtype.
    pub(crate) fn index_constraint(
        &mut self,
        scope: ScopeId,
        r: &ast::DiscreteRange,
    ) -> Result<(RangeC, SubtypeId), Reported> {
        // Array type bounds are required (Q5): overflow there is `E1401`.
        self.required_ctx = true;
        let res = self.discrete_range(scope, r, None);
        self.required_ctx = false;
        let (rc, t) = res?;
        // 5.3.2.1: the index subtype is the type mark of the discrete range
        // (`byte range 3 to 4`, `mid`), else the range's type.
        let ix = match (r, &rc) {
            (
                ast::DiscreteRange::Range(ast::Range::Name(n)),
                RangeC::Attr {
                    prefix: AttrPrefix::Subtype(st),
                    ..
                },
            ) if !is_attribute_name(n) => *st,
            (
                ast::DiscreteRange::Subtype(_),
                RangeC::Attr {
                    prefix: AttrPrefix::Subtype(st),
                    ..
                },
            ) => self.subtypes[*st].parent.unwrap_or(*st),
            _ => self.types[t].first_subtype,
        };
        Ok((rc, ix))
    }

    /// A type mark that must be discrete (array index subtypes).
    fn discrete_type_mark(&mut self, scope: ScopeId, n: &ast::Name) -> Result<SubtypeId, Reported> {
        let st = self.type_mark(scope, n)?;
        let k = &self.types[self.base(st)].kind;
        if !k.is_discrete() && !matches!(k, TypeKind::Error | TypeKind::Generic { .. }) {
            return Err(self.error(
                Diagnostic::error(diag::NOT_A_TYPE, "an index subtype must be discrete")
                    .with_primary(n.span, "not an integer or enumeration type"),
            ));
        }
        Ok(st)
    }

    /// A discrete range: `l to r`, a type mark, `t range l to r`. Returns the range and its type.
    /// `want` is the index type when known (index constraints); otherwise bounds are resolved
    /// together and two universal integers give INTEGER (5.3.2.2).
    pub(crate) fn discrete_range(
        &mut self,
        scope: ScopeId,
        r: &ast::DiscreteRange,
        want: Option<TypeId>,
    ) -> Result<(RangeC, TypeId), Reported> {
        match r {
            ast::DiscreteRange::Range(ast::Range::Simple { left, dir, right }) => {
                let t = match want {
                    Some(t) => t,
                    None => self.range_type_of(scope, left, right)?,
                };
                let req = self.required_ctx;
                let l = self.complete_context(scope, left, Expected::Type(t), req);
                if self.is_poison_expr(l) {
                    return Err(Reported);
                }
                let rr = self.complete_context(scope, right, Expected::Type(t), req);
                Ok((
                    RangeC::Explicit {
                        left: l,
                        dir: *dir,
                        right: rr,
                    },
                    t,
                ))
            }
            ast::DiscreteRange::Range(ast::Range::Name(n)) => {
                if let Some(r) = self.range_attr(scope, n) {
                    let (rc, t) = r?;
                    if let Some(w) = want
                        && w != t
                        && w != self.poison.ty
                        && t != self.poison.ty
                    {
                        return Err(self.error(
                            Diagnostic::error(diag::TYPE_MISMATCH, {
                                let n = self.type_names(&[w, t]);
                                format!("expected a range of `{}`, found one of `{}`", n[0], n[1])
                            })
                            .with_primary(n.span, "wrong index type"),
                        ));
                    }
                    return Ok((rc, t));
                }
                if let Some(r) = self.range_record_object(scope, n) {
                    return r;
                }
                let st = self.discrete_type_mark(scope, n)?;
                let t = self.base(st);
                Ok((
                    RangeC::Attr {
                        prefix: AttrPrefix::Subtype(st),
                        reverse: false,
                        dim: 1,
                    },
                    t,
                ))
            }
            ast::DiscreteRange::Subtype(si) => {
                let st = self.subtype_indication(scope, si)?;
                let t = self.base(st);
                Ok((
                    RangeC::Attr {
                        prefix: AttrPrefix::Subtype(st),
                        reverse: false,
                        dim: 1,
                    },
                    t,
                ))
            }
        }
    }

    /// 2019 (LCS-2016-099): a constant of a range record type (`R'RECORD`) used as a range,
    /// `bit_vector(r)`. `None` when `n` is not such an object.
    // ponytail: only a constant whose value folds at analysis; a run-time range record needs
    // a range with a dynamic direction (a new `RangeC` form in lowering and elaboration)
    fn range_record_object(
        &mut self,
        scope: ScopeId,
        n: &ast::Name,
    ) -> Option<Result<(RangeC, TypeId), Reported>> {
        if self.file_rev() < yavs_syntax::Std::Vhdl2019 {
            return None;
        }
        let crate::expr::names::NameRes::One(d) = self.classify(scope, n) else {
            return None;
        };
        let DeclKind::Object(o) = &self.decls[d].kind else {
            return None;
        };
        let (class, default) = (o.class, o.default);
        let rb = self.base(o.subtype);
        let (&t, _) = self
            .range_records
            .iter()
            .find(|&(_, &st)| self.base(st) == rb)?;
        let value = default.and_then(|e| self.value(e).cloned());
        let Some(ConstValue::Record(f)) =
            value.filter(|_| class == crate::decl::ObjClass::Constant)
        else {
            return Some(Err(self.error(
                Diagnostic::error(
                    diag::NOT_STATIC,
                    "a range given by an object of a range record type needs a constant with a static value",
                )
                .with_primary(n.span, "not a constant whose value is known at analysis"),
            )));
        };
        let [l, r, ConstValue::Enum(dir)] = &*f else {
            return Some(Err(self.ice(n.span, "malformed range record value")));
        };
        let st = self.types[t].first_subtype;
        let (l, r) = (
            self.const_expr(l.clone(), st, n.span),
            self.const_expr(r.clone(), st, n.span),
        );
        let dir = if *dir == 0 {
            Direction::To
        } else {
            Direction::Downto
        };
        Some(Ok((
            RangeC::Explicit {
                left: l,
                dir,
                right: r,
            },
            t,
        )))
    }

    /// The type of `l to r` without a type mark.
    fn range_type_of(
        &mut self,
        scope: ScopeId,
        l: &ast::Expr,
        r: &ast::Expr,
    ) -> Result<TypeId, Reported> {
        let a = self.discrete_cands(scope, l);
        let b = self.discrete_cands(scope, r);
        let ui = self.well_known.universal_integer;
        let both: Vec<TypeId> = a.iter().copied().filter(|t| b.contains(t)).collect();
        let pick = match both.as_slice() {
            [t] if Some(*t) == ui => self.well_known.integer,
            [t] => Some(*t),
            _ => {
                // A universal bound adapts to the other bound's specific type.
                let a_s: Vec<TypeId> = a.iter().copied().filter(|t| Some(*t) != ui).collect();
                let b_s: Vec<TypeId> = b.iter().copied().filter(|t| Some(*t) != ui).collect();
                match (
                    a_s.as_slice(),
                    b_s.as_slice(),
                    a.contains(&ui.unwrap_or(self.poison.ty)),
                    b.contains(&ui.unwrap_or(self.poison.ty)),
                ) {
                    ([t], [u], _, _) if t == u => Some(*t),
                    // Only to an integer type (9.3.6 implicit conversion); else the universal
                    // bound makes the range `integer` and the other bound is the mismatch.
                    ([t], [], _, true) | ([], [t], true, _)
                        if matches!(self.types[*t].kind, TypeKind::Integer) =>
                    {
                        Some(*t)
                    }
                    ([_], [], _, true) | ([], [_], true, _) => self.well_known.integer,
                    _ if both.len() > 1 => None,
                    _ => None,
                }
            }
        };
        match pick {
            Some(t) => Ok(t),
            None if a.contains(&self.poison.ty) || b.contains(&self.poison.ty) => Err(Reported),
            None => Err(self.error(
                Diagnostic::error(
                    diag::TYPE_MISMATCH,
                    "the type of this range cannot be determined",
                )
                .with_primary(l.span.to(r.span), "bounds of no single discrete type")
                .with_help("use a type mark: `t range l to r`"),
            )),
        }
    }

    /// `subtype S is ...;`
    pub(crate) fn subtype_decl(
        &mut self,
        scope: ScopeId,
        sd: &ast::SubtypeDecl,
    ) -> Result<(), Reported> {
        let st = self.subtype_indication(scope, &sd.subtype)?;
        // A subtype declaration always names a new subtype entity, even without a constraint.
        let d = self.subtypes[st].clone_def(st, sd.name, sd.span);
        let st = self.subtypes.push(d);
        let decl = self.new_decl(
            scope,
            Designator::Ident(sd.name),
            DeclKind::Subtype(st),
            sd.span,
        );
        self.declare(scope, decl);
        self.register_well_known_subtype(scope, sd.name, st);
        Ok(())
    }

    /// A subtype indication (6.3): resolution, type mark and constraint.
    pub(crate) fn subtype_indication(
        &mut self,
        scope: ScopeId,
        si: &ast::SubtypeIndication,
    ) -> Result<SubtypeId, Reported> {
        let mark = self.type_mark(scope, &si.type_mark)?;
        if si.resolution.is_none() && si.constraint.is_none() {
            return Ok(mark);
        }
        let base = self.base(mark);
        let resolution = match &si.resolution {
            None => self.subtypes[mark].resolution.clone(),
            Some(r) => Some(self.resolution(scope, r, base)?),
        };
        let constraint = match &si.constraint {
            None => self.subtypes[mark].constraint.clone(),
            Some(c) => self.constraint(scope, c, mark)?,
        };
        Ok(self.subtypes.push(SubtypeDef {
            base,
            name: None,
            parent: Some(mark),
            constraint,
            resolution,
            span: si.span,
        }))
    }

    /// A resolution indication (6.3): a function name, `(f)` for array elements,
    /// `(a f, b g)` per record element.
    fn resolution(
        &mut self,
        scope: ScopeId,
        r: &ast::Resolution,
        base: TypeId,
    ) -> Result<Resolution, Reported> {
        match r {
            ast::Resolution::Function(n) => {
                let cands = match self.classify(scope, n) {
                    crate::expr::names::NameRes::Overloads(ds) => ds,
                    crate::expr::names::NameRes::One(d) => vec![d],
                    other => return Err(self.report_name(scope, other, n.span, "a function name")),
                };
                let fs: Vec<DeclId> = cands
                    .into_iter()
                    .map(|d| self.entity(d))
                    .filter(|&d| match self.decls[d].kind.subprogram() {
                        Some(s) => {
                            s.params.len() == 1
                                && s.ret.is_some_and(|r| {
                                    self.base(r) == base
                                        || matches!(self.types[base].kind, TypeKind::Error)
                                })
                        }
                        None => false,
                    })
                    .collect();
                match fs.as_slice() {
                    [f] => Ok(Resolution::Function(*f)),
                    [] => Err(self.error(
                        Diagnostic::error(
                            diag::BAD_RESOLUTION,
                            format!(
                                "no resolution function for `{}` is named here",
                                self.type_name(base)
                            ),
                        )
                        .with_primary(
                            n.span,
                            "expected a function of one parameter returning the resolved type",
                        ),
                    )),
                    _ => Err(self.error(
                        Diagnostic::error(
                            diag::BAD_RESOLUTION,
                            "the resolution function name is ambiguous",
                        )
                        .with_primary(n.span, "several functions fit"),
                    )),
                }
            }
            ast::Resolution::Element(inner) => {
                let TypeKind::Array { elem, .. } = &self.types[base].kind else {
                    return Err(self.error(
                        Diagnostic::error(
                            diag::BAD_RESOLUTION,
                            "an element resolution `(f)` needs an array type",
                        )
                        .with_primary(resolution_span(r), "the subtype is not an array"),
                    ));
                };
                let eb = self.base(*elem);
                Ok(Resolution::Element(Box::new(
                    self.resolution(scope, inner, eb)?,
                )))
            }
            ast::Resolution::Record(items) => {
                let mut out = Vec::new();
                for (name, res) in items {
                    let Some(i) = self.record_elem(base, name.key) else {
                        return Err(self.error(
                            Diagnostic::error(
                                diag::BAD_RECORD_CONSTRAINT,
                                format!(
                                    "`{}` is not an element of `{}`",
                                    name.text,
                                    self.type_name(base)
                                ),
                            )
                            .with_primary(name.span, "no such element"),
                        ));
                    };
                    let eb = match &self.types[base].kind {
                        TypeKind::Record { elems } => self.base(elems[i as usize].subtype),
                        _ => self.poison.ty,
                    };
                    out.push((i, self.resolution(scope, res, eb)?));
                }
                Ok(Resolution::Record(out.into()))
            }
        }
    }

    fn constraint(
        &mut self,
        scope: ScopeId,
        c: &ast::Constraint,
        mark: SubtypeId,
    ) -> Result<Constraint, Reported> {
        let base = self.base(mark);
        let existing = self.subtypes[mark].constraint.clone();
        let res = self.merge_constraint(scope, c, base, &existing)?;
        if let (ast::Constraint::Range(r), Constraint::Range(rc)) = (c, &res) {
            self.check_compatible(rc, mark, range_span(r), "type mark")?;
        }
        Ok(res)
    }

    /// `E1215`: range `rc` has static bounds and is not null, and a bound lies outside the
    /// static range of `outer`, the type mark or index subtype it constrains (5.2.1, 5.3.2.2, 6.3).
    pub(crate) fn check_compatible(
        &mut self,
        rc: &RangeC,
        outer: SubtypeId,
        span: Span,
        what: &str,
    ) -> Result<(), Reported> {
        use crate::expr::aggregate::{range_value, subtype_range};
        let (Some(r), Some(o)) = (range_value(self, rc), subtype_range(self, outer, 0)) else {
            return Ok(());
        };
        if r.is_null() || o.is_null() || (o.contains(r.low()) && o.contains(r.high())) {
            return Ok(());
        }
        let name = self.subtype_name(outer);
        Err(self.error(
            Diagnostic::error(
                diag::CONSTRAINT_OUTSIDE,
                format!("the range is not inside {what} `{name}`"),
            )
            .with_primary(span, format!("{} to {} is outside {} to {}", r.low(), r.high(), o.low(), o.high()))
            .with_note("5.2.1, 5.3.2.2: every bound of a non-null range constraint must belong to the subtype it constrains"),
        ))
    }

    /// Apply constraint `c` to type `t` whose subtype already has `existing`
    /// (2008 5.3.2.2, 6.3): `open` keeps a dimension unconstrained, constraining a
    /// constrained dimension or element is `E1210`. Merged once, here (rule 1).
    fn merge_constraint(
        &mut self,
        scope: ScopeId,
        c: &ast::Constraint,
        t: TypeId,
        existing: &Constraint,
    ) -> Result<Constraint, Reported> {
        let kind_err =
            |db: &mut Db, span: Span, msg: &str, label: &str| -> Result<Constraint, Reported> {
                if matches!(db.types[t].kind, TypeKind::Error) {
                    return Err(Reported);
                }
                Err(db.error(
                    Diagnostic::error(diag::BAD_CONSTRAINT, msg.to_string())
                        .with_primary(span, label.to_string()),
                ))
            };
        match c {
            ast::Constraint::Range(r) => {
                if !self.types[t].kind.is_scalar() {
                    return kind_err(
                        self,
                        range_kw_span(self, r),
                        "a range constraint needs a scalar type",
                        "range constraint on a composite type",
                    );
                }
                Ok(Constraint::Range(self.range(scope, r, t)?))
            }
            ast::Constraint::Array {
                indexes,
                element,
                span,
            } => {
                // 5.4, 6.3: an index constraint on an access type applies to the
                // designated array type; the merged constraint is stored on the
                // access subtype.
                let t = match &self.types[t].kind {
                    TypeKind::Access { designated }
                        if matches!(
                            self.types[self.base(*designated)].kind,
                            TypeKind::Array { .. }
                        ) =>
                    {
                        self.base(*designated)
                    }
                    _ => t,
                };
                let TypeKind::Array { indexes: idx, elem } = &self.types[t].kind else {
                    return kind_err(
                        self,
                        *span,
                        "an index constraint needs an array type",
                        "index constraint on a non-array type",
                    );
                };
                let (idx, elem) = (idx.clone(), *elem);
                let (old_dims, old_elem) = match existing {
                    Constraint::Array { dims, elem } => (dims.clone(), (**elem).clone()),
                    _ => (None, Constraint::None),
                };
                let dims = match indexes {
                    None => old_dims,
                    Some(rs) if old_dims.is_some() => {
                        let at = match rs.first() {
                            Some(ast::DiscreteRange::Range(r)) => range_span(r),
                            Some(ast::DiscreteRange::Subtype(si)) => si.span,
                            None => *span,
                        };
                        return Err(self.error(
                            Diagnostic::error(
                                diag::CONSTRAINED_TWICE,
                                "the index ranges of this array subtype are already constrained",
                            )
                            .with_primary(at, "second index constraint"),
                        ));
                    }
                    Some(rs) => {
                        if rs.len() != idx.len() {
                            return Err(self.error(
                                Diagnostic::error(
                                    diag::INDEX_ARITY,
                                    format!(
                                        "the array has {} dimension(s), the constraint {}",
                                        idx.len(),
                                        rs.len()
                                    ),
                                )
                                .with_primary(*span, "wrong number of index ranges"),
                            ));
                        }
                        let mut dims = Vec::new();
                        for (r, &i) in rs.iter().zip(idx.iter()) {
                            let it = self.base(i);
                            let rc = self.discrete_range(scope, r, Some(it))?.0;
                            let at = match r {
                                ast::DiscreteRange::Range(r) => range_span(r),
                                ast::DiscreteRange::Subtype(si) => si.span,
                            };
                            self.check_compatible(&rc, i, at, "index subtype")?;
                            dims.push(rc);
                        }
                        Some(dims.into_boxed_slice())
                    }
                };
                let elem_c = match element {
                    None => old_elem,
                    Some(ec) => {
                        let current = self.effective_elem(&old_elem, elem);
                        let et = self.base(elem);
                        self.merge_constraint(scope, ec, et, &current)?
                    }
                };
                Ok(Constraint::Array {
                    dims,
                    elem: Box::new(elem_c),
                })
            }
            ast::Constraint::Record(items, span) => {
                let TypeKind::Record { elems } = &self.types[t].kind else {
                    return kind_err(
                        self,
                        *span,
                        "a record constraint needs a record type",
                        "record constraint on a non-record type",
                    );
                };
                let decl_elems: Vec<SubtypeId> = elems.iter().map(|e| e.subtype).collect();
                let mut out: Vec<(u32, Constraint)> = match existing {
                    Constraint::Record { elems } => elems.to_vec(),
                    _ => Vec::new(),
                };
                let mut seen: Vec<u32> = Vec::new();
                for (name, ec) in items {
                    let Some(i) = self.record_elem(t, name.key) else {
                        return Err(self.error(
                            Diagnostic::error(
                                diag::BAD_RECORD_CONSTRAINT,
                                format!(
                                    "`{}` is not an element of `{}`",
                                    name.text,
                                    self.type_name(t)
                                ),
                            )
                            .with_primary(name.span, "no such element"),
                        ));
                    };
                    if seen.contains(&i) {
                        return Err(self.error(
                            Diagnostic::error(
                                diag::BAD_RECORD_CONSTRAINT,
                                format!("element `{}` is constrained twice", name.text),
                            )
                            .with_primary(name.span, "repeated here"),
                        ));
                    }
                    seen.push(i);
                    let est = decl_elems[i as usize];
                    let current = match out.iter().find(|(k, _)| *k == i) {
                        Some((_, c)) => c.clone(),
                        None => self.subtypes[est].constraint.clone(),
                    };
                    let et = self.base(est);
                    if self.types[et].kind.is_scalar() {
                        return Err(self.error(
                            Diagnostic::error(
                                diag::BAD_RECORD_CONSTRAINT,
                                format!(
                                    "element `{}` is scalar and cannot be constrained here",
                                    name.text
                                ),
                            )
                            .with_primary(name.span, "not a composite element"),
                        ));
                    }
                    let merged = self.merge_constraint(scope, ec, et, &current)?;
                    out.retain(|(k, _)| *k != i);
                    out.push((i, merged));
                }
                out.sort_by_key(|(k, _)| *k);
                Ok(Constraint::Record { elems: out.into() })
            }
        }
    }

    /// The constraint of an array element: what the array subtype says, else the
    /// element subtype's own.
    fn effective_elem(&self, c: &Constraint, elem: SubtypeId) -> Constraint {
        match c {
            Constraint::None => self.subtypes[elem].constraint.clone(),
            c => c.clone(),
        }
    }

    /// A range constraint of type `base`.
    /// The access, file or (with `protected`) protected type in `t`, through composites
    /// (5.5, 6.4.2.2).
    pub(crate) fn bad_elem(&self, t: TypeId, protected: bool) -> Option<(&'static str, TypeId)> {
        match &self.types[t].kind {
            TypeKind::Access { .. } => Some(("access", t)),
            TypeKind::File { .. } => Some(("file", t)),
            TypeKind::Protected { .. } if protected => Some(("protected", t)),
            TypeKind::Array { elem, .. } => self.bad_elem(self.base(*elem), protected),
            TypeKind::Record { elems } => elems
                .iter()
                .find_map(|e| self.bad_elem(self.base(e.subtype), protected)),
            _ => None,
        }
    }

    /// `E1223` when a constant, signal, generic, port or constant or signal parameter
    /// (`what`) would be of an access or file type or contain one (6.4.2.2, 6.4.2.3, 6.5.2).
    /// Protected types are `check_not_protected`'s.
    pub(crate) fn check_not_access_or_file(
        &mut self,
        what: &str,
        st: SubtypeId,
        span: Span,
    ) -> Result<(), Reported> {
        let t = self.base(st);
        let Some((kind, bad)) = self.bad_elem(t, false) else {
            return Ok(());
        };
        let msg = if bad == t {
            format!("a {what} cannot be of {kind} type `{}`", self.type_name(t))
        } else {
            format!(
                "a {what} cannot be of type `{}`, which contains {kind} type `{}`",
                self.type_name(t),
                self.type_name(bad)
            )
        };
        Err(self.error(
            Diagnostic::error(diag::BAD_OBJECT_TYPE, msg)
                .with_primary(span, format!("{what} of {kind} type"))
                .with_note("LRM 6.4.2.2, 6.4.2.3, 6.5.2, 5.6.2: access values live only in variables, files only in file objects; 2008 methods of a protected type take and return neither")
                .with_help(if what.starts_with("protected method") {
                    "keep the access or file part inside the protected object, or analyze as VHDL-2019"
                } else if kind == "access" {
                    "use a variable, or a variable parameter"
                } else {
                    "declare a file object, or a file parameter"
                }),
        ))
    }

    pub(crate) fn range(
        &mut self,
        scope: ScopeId,
        r: &ast::Range,
        base: TypeId,
    ) -> Result<RangeC, Reported> {
        match r {
            ast::Range::Simple { left, dir, right } => {
                let l = self.expr(scope, left, Expected::Type(base));
                // One wrong-typed range is one error (`integer range red to green`).
                if self.is_poison_expr(l) {
                    return Err(Reported);
                }
                let rr = self.expr(scope, right, Expected::Type(base));
                Ok(RangeC::Explicit {
                    left: l,
                    dir: *dir,
                    right: rr,
                })
            }
            ast::Range::Name(n) => self.range_name(scope, n, base),
        }
    }

    /// A range written as a name: a range attribute, or a type mark for its range.
    pub(crate) fn range_name(
        &mut self,
        scope: ScopeId,
        n: &ast::Name,
        base: TypeId,
    ) -> Result<RangeC, Reported> {
        match self.range_attr(scope, n) {
            Some(r) => {
                let (rc, t) = r?;
                if t != base && t != self.poison.ty && base != self.poison.ty {
                    return Err(self.error(
                        Diagnostic::error(diag::TYPE_MISMATCH, {
                            let n = self.type_names(&[base, t]);
                            format!("expected a range of `{}`, found one of `{}`", n[0], n[1])
                        })
                        .with_primary(n.span, "wrong type"),
                    ));
                }
                Ok(rc)
            }
            None => {
                if let Some(r) = self.range_record_object(scope, n) {
                    let (rc, t) = r?;
                    if t != base && t != self.poison.ty && base != self.poison.ty {
                        let names = self.type_names(&[base, t]);
                        return Err(self.error(
                            Diagnostic::error(
                                diag::TYPE_MISMATCH,
                                format!(
                                    "expected a range of `{}`, found one of `{}`",
                                    names[0], names[1]
                                ),
                            )
                            .with_primary(n.span, "wrong type"),
                        ));
                    }
                    return Ok(rc);
                }
                let st = self.type_mark(scope, n)?;
                Ok(RangeC::Attr {
                    prefix: AttrPrefix::Subtype(st),
                    reverse: false,
                    dim: 1,
                })
            }
        }
    }

    /// The types of an attribute's object prefix. No interpretation at all means an
    /// undeclared or failed name: that is reported as such (or stays silent, 8.2),
    /// never as a wrong prefix.
    fn attr_prefix_types(
        &mut self,
        scope: ScopeId,
        prefix: &ast::Name,
    ) -> Result<Vec<TypeId>, Reported> {
        let ts = self.prefix_types(scope, prefix).map_err(|_| Reported)?;
        if ts.is_empty() {
            use crate::expr::names::NameRes;
            let r = self.classify(scope, prefix);
            if !matches!(r, NameRes::One(_) | NameRes::Overloads(_) | NameRes::Value) {
                return Err(self.report_name(scope, r, prefix.span, "a name"));
            }
        }
        Ok(ts)
    }

    /// `a'range`, `t'reverse_range(2)` (16.2): `None` when `n` is not a range attribute.
    pub(crate) fn range_attr(
        &mut self,
        scope: ScopeId,
        n: &ast::Name,
    ) -> Option<Result<(RangeC, TypeId), Reported>> {
        let (prefix, attr, dim_arg) = match &n.kind {
            ast::NameKind::Attribute { prefix, attr } => (prefix.as_ref(), *attr, None),
            ast::NameKind::Apply { prefix, args } => match (&prefix.kind, args.as_slice()) {
                (
                    ast::NameKind::Attribute { prefix, attr },
                    [
                        ast::AssocElem {
                            formal: None,
                            actual: ast::Actual::Expr(x),
                            ..
                        },
                    ],
                ) => (prefix.as_ref(), *attr, Some(x)),
                _ => return None,
            },
            _ => return None,
        };
        let reverse = match attr.key.as_str() {
            "range" => false,
            "reverse_range" => true,
            _ => return None,
        };
        Some(self.range_attr_inner(scope, prefix, attr, reverse, dim_arg, n.span))
    }

    pub(crate) fn range_attr_inner(
        &mut self,
        scope: ScopeId,
        prefix: &ast::Name,
        attr: Ident,
        reverse: bool,
        dim_arg: Option<&ast::Expr>,
        span: Span,
    ) -> Result<(RangeC, TypeId), Reported> {
        let dim = match dim_arg {
            None => 1,
            Some(x) => {
                let e = self.static_expr(scope, x, Expected::Any);
                match self.value(e).and_then(|v| v.as_int()) {
                    Some(d) if d >= 1 => d as u32,
                    _ if self.is_poison_expr(e) => return Err(Reported),
                    _ => return Err(self.error(
                        Diagnostic::error(diag::NOT_STATIC, "the dimension of a range attribute must be a locally static positive integer")
                            .with_primary(x.span, "not a static dimension"),
                    )),
                }
            }
        };
        let (aprefix, t) = if let Some(st) = self.quiet_type_mark(scope, prefix) {
            (AttrPrefix::Subtype(st), self.base(st))
        } else {
            let ts = self.attr_prefix_types(scope, prefix)?;
            let arrays: Vec<TypeId> = self
                .with_designated(ts)
                .into_iter()
                .filter(|&t| matches!(self.types[t].kind, TypeKind::Array { .. }))
                .collect();
            let [t] = arrays.as_slice() else {
                // A prefix that cannot take the attribute at all (`a'range` of a scalar
                // object) is E1621 at the attribute, like every other attribute.
                if self.report_attr(scope, prefix, attr) {
                    return Err(Reported);
                }
                return Err(self.error(
                    Diagnostic::error(diag::NOT_AN_OBJECT, "the prefix of `'range` must be an array object or an array or scalar type mark")
                        .with_primary(prefix.span, "not an array"),
                ));
            };
            let p = self.resolve_prefix(scope, prefix, *t);
            self.not_read(p);
            (AttrPrefix::Expr(p), *t)
        };
        let index = match &self.types[t].kind {
            TypeKind::Array { indexes, .. } => match indexes.get(dim as usize - 1) {
                Some(&i) => self.base(i),
                None => {
                    return Err(self.error(
                        Diagnostic::error(
                            diag::ATTR_DIMENSION,
                            format!("the array has {} dimension(s), not {dim}", indexes.len()),
                        )
                        .with_primary(span, "no such dimension"),
                    ));
                }
            },
            // Permissive (2019 allows `T'range` of a scalar subtype): its own range.
            k if k.is_scalar() => t,
            TypeKind::Error => t,
            _ => {
                return Err(self.error(
                    Diagnostic::error(
                        diag::NOT_AN_OBJECT,
                        "`'range` needs an array or scalar prefix",
                    )
                    .with_primary(prefix.span, "not an array"),
                ));
            }
        };
        Ok((
            RangeC::Attr {
                prefix: aprefix,
                reverse,
                dim,
            },
            index,
        ))
    }

    /// A name that must denote a type or subtype.
    pub(crate) fn type_mark(
        &mut self,
        scope: ScopeId,
        n: &ast::Name,
    ) -> Result<SubtypeId, Reported> {
        match &n.kind {
            ast::NameKind::Attribute { prefix, attr } => {
                return self.attribute_type_mark(scope, prefix, *attr);
            }
            // 2019 `A'INDEX(n)`, the only type mark with a parameter.
            ast::NameKind::Apply { prefix, args } => {
                if let ast::NameKind::Attribute { prefix, attr } = &prefix.kind
                    && attr.key.as_str() == "index"
                    && let [
                        ast::AssocElem {
                            formal: None,
                            actual: ast::Actual::Expr(x),
                            ..
                        },
                    ] = args.as_slice()
                {
                    return self.index_type_mark(scope, prefix, *attr, Some(x), n.span);
                }
            }
            _ => {}
        }
        let d = self.expanded_name_decl(scope, n)?;
        match self.decls[self.entity(d)].kind {
            DeclKind::Type(t) => {
                if let TypeKind::Incomplete {
                    completion: Some(c),
                } = self.types[t].kind
                {
                    return Ok(self.types[c].first_subtype);
                }
                self.check_incomplete_use(scope, t, n.span)?;
                Ok(self.types[t].first_subtype)
            }
            DeclKind::Subtype(s) => Ok(s),
            DeclKind::GenericType(t) => Ok(self.types[t].first_subtype),
            DeclKind::Error => Err(Reported),
            _ => Err(self.error(
                Diagnostic::error(
                    diag::NOT_A_TYPE,
                    format!("`{}` is not a type or subtype", self.decls[d].name),
                )
                .with_primary(n.span, "expected a type mark")
                .with_secondary(self.decls[d].span, "declared here"),
            )),
        }
    }

    /// The subtype of the object `e`; when it is not fully constrained, a subtype whose bounds
    /// come from the object at run time (rule 1).
    pub(crate) fn runtime_subtype(&mut self, e: ExprId, span: Span) -> SubtypeId {
        let st = self.exprs[e].subtype;
        if self.constrainedness(st) == Constrainedness::Full {
            return st;
        }
        let base = self.base(st);
        self.subtypes.push(SubtypeDef {
            base,
            name: None,
            parent: Some(st),
            constraint: Constraint::FromObject {
                expr: e,
                elem_depth: 0,
            },
            resolution: self.subtypes[st].resolution.clone(),
            span,
        })
    }

    /// `x'subtype`, `x'element`, `t'base` used as type marks (16.2).
    pub(crate) fn attribute_type_mark(
        &mut self,
        scope: ScopeId,
        prefix: &ast::Name,
        attr: Ident,
    ) -> Result<SubtypeId, Reported> {
        let what = attr.key.as_str();
        match what {
            "index" => return self.index_type_mark(scope, prefix, attr, None, attr.span),
            "record" => return self.range_record_type_mark(scope, prefix, attr),
            _ => {}
        }
        let st = if let Some(st) = self.quiet_type_mark(scope, prefix) {
            st
        } else if matches!(what, "subtype" | "element") {
            let ts = self.attr_prefix_types(scope, prefix)?;
            let [t] = ts.as_slice() else {
                return Err(self.error(
                    Diagnostic::error(
                        diag::NOT_AN_OBJECT,
                        format!("the prefix of `'{}` must denote an object", attr.text),
                    )
                    .with_primary(prefix.span, "not an object of one type"),
                ));
            };
            let e = self.resolve_prefix(scope, prefix, *t);
            self.not_read(e);
            self.runtime_subtype(e, attr.span)
        } else {
            return Err(self.error(
                Diagnostic::error(
                    diag::NOT_A_TYPE,
                    format!("`'{}` does not denote a subtype", attr.text),
                )
                .with_primary(attr.span, "not a type mark"),
            ));
        };
        match what {
            "subtype" => Ok(st),
            "designated_subtype" => self.designated_subtype_attr(st, attr),
            "base" => {
                let t = self.base(st);
                Ok(self.types[t].base_subtype)
            }
            "element" => match self.element_subtype(match self.subtypes[st].constraint {
                Constraint::FromObject { .. } => self.subtypes[st].parent.unwrap_or(st),
                _ => st,
            }) {
                Some(e) => match self.subtypes[st].constraint {
                    // A constrained element subtype (`ow(open)(7 downto 0)`) is the formal's,
                    // not the object's.
                    Constraint::FromObject { .. }
                        if self.constrainedness(e) == Constrainedness::Full =>
                    {
                        Ok(e)
                    }
                    Constraint::FromObject { expr, elem_depth } => {
                        let base = self.base(e);
                        Ok(self.subtypes.push(SubtypeDef {
                            base,
                            name: None,
                            parent: Some(e),
                            constraint: Constraint::FromObject {
                                expr,
                                elem_depth: elem_depth + 1,
                            },
                            resolution: self.subtypes[e].resolution.clone(),
                            span: attr.span,
                        }))
                    }
                    _ => Ok(e),
                },
                None => Err(self.error(
                    Diagnostic::error(diag::NOT_A_TYPE, "`'element` needs an array prefix")
                        .with_primary(prefix.span, "not an array"),
                )),
            },
            _ => Err(self.error(
                Diagnostic::error(
                    diag::NOT_A_TYPE,
                    format!("`'{}` does not denote a subtype", attr.text),
                )
                .with_primary(attr.span, "not a type mark"),
            )),
        }
    }

    // -- queries ------------------------------------------------------------

    /// The scalar range of a subtype: its own constraint, else the type's base range.
    pub fn scalar_range(&self, st: SubtypeId) -> Option<RangeC> {
        match &self.subtypes[st].constraint {
            Constraint::Range(r) => Some(r.clone()),
            // A scalar named return value's subtype (2019, 4.2.1): the hidden object's range,
            // which M2 passes as a one-dimensional descriptor.
            &Constraint::FromObject {
                expr,
                elem_depth: 0,
            } if self.types[self.base(st)].kind.is_scalar() => Some(RangeC::Attr {
                prefix: crate::hir::AttrPrefix::Expr(expr),
                reverse: false,
                dim: 1,
            }),
            Constraint::None => {
                let t = self.base(st);
                let b = self.types[t].base_subtype;
                match &self.subtypes[b].constraint {
                    Constraint::Range(r) if b != st => Some(r.clone()),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Folded `(left, dir, right)` of a scalar subtype, when static. Enumeration
    /// types without a constraint span all their literals.
    pub fn static_bounds(&self, st: SubtypeId) -> Option<(ConstValue, Direction, ConstValue)> {
        match self.scalar_range(st) {
            Some(RangeC::Explicit { left, dir, right }) => {
                Some((self.value(left)?.clone(), dir, self.value(right)?.clone()))
            }
            Some(RangeC::Attr { .. }) => None,
            None => match &self.types[self.base(st)].kind {
                TypeKind::Enum { lits } => Some((
                    ConstValue::Enum(0),
                    Direction::To,
                    ConstValue::Enum(lits.len().checked_sub(1)? as u32),
                )),
                _ => None,
            },
        }
    }

    /// `t(r1, r2)` written as a name, where a subtype indication is wanted but the
    /// parser saw an expression (a generic type actual `t => slv(n - 1 downto 0)`):
    /// `None` when `n` is not an index-constrained type mark.
    pub(crate) fn index_constrained_name(
        &mut self,
        scope: ScopeId,
        n: &ast::Name,
    ) -> Option<Result<SubtypeId, Reported>> {
        let ast::NameKind::Apply { prefix, args } = &n.kind else {
            return None;
        };
        let mark = self.quiet_type_mark(scope, prefix)?;
        let base = self.base(mark);
        let TypeKind::Array { indexes, .. } = &self.types[base].kind else {
            return None;
        };
        let indexes = indexes.clone();
        Some((|| {
            let (old_dims, elem) = match &self.subtypes[mark].constraint {
                Constraint::Array { dims, elem } => (dims.clone(), (**elem).clone()),
                _ => (None, Constraint::None),
            };
            if old_dims.is_some() {
                return Err(self.error(
                    Diagnostic::error(
                        diag::CONSTRAINED_TWICE,
                        "the index ranges of this array subtype are already constrained",
                    )
                    .with_primary(n.span, "second index constraint"),
                ));
            }
            if args.len() != indexes.len() {
                return Err(self.error(
                    Diagnostic::error(
                        diag::INDEX_ARITY,
                        format!(
                            "the array has {} dimension(s), the constraint {}",
                            indexes.len(),
                            args.len()
                        ),
                    )
                    .with_primary(n.span, "wrong number of index ranges"),
                ));
            }
            let mut dims = Vec::with_capacity(args.len());
            for (a, &i) in args.iter().zip(indexes.iter()) {
                let it = self.base(i);
                dims.push(match &a.actual {
                    ast::Actual::Range(r) => self.range(scope, r, it)?,
                    ast::Actual::Expr(ast::Expr {
                        kind: ast::ExprKind::Name(rn),
                        ..
                    }) => self.range_name(scope, rn, it)?,
                    ast::Actual::Subtype(si) => {
                        let st = self.subtype_indication(scope, si)?;
                        RangeC::Attr {
                            prefix: AttrPrefix::Subtype(st),
                            reverse: false,
                            dim: 1,
                        }
                    }
                    _ => {
                        return Err(self.error(
                            Diagnostic::error(
                                diag::BAD_CONSTRAINT,
                                "an index constraint needs a range",
                            )
                            .with_primary(a.span, "not a range"),
                        ));
                    }
                });
            }
            Ok(self.subtypes.push(SubtypeDef {
                base,
                name: None,
                parent: Some(mark),
                constraint: Constraint::Array {
                    dims: Some(dims.into()),
                    elem: Box::new(elem),
                },
                resolution: self.subtypes[mark].resolution.clone(),
                span: n.span,
            }))
        })())
    }

    /// The subtype of element `elem` of a record object of subtype `st`: the
    /// element's declared subtype with the record constraint of `st` on it (6.3).
    pub(crate) fn field_subtype(&mut self, st: SubtypeId, elem: u32) -> SubtypeId {
        let TypeKind::Record { elems } = &self.types[self.base(st)].kind else {
            return self.poison.subtype;
        };
        let declared = elems[elem as usize].subtype;
        // `r'subtype` of a record object: the element's bounds are the object's own, read at
        // run time from `r.elem` (LRM 16.2.2).
        if let Constraint::FromObject {
            expr,
            elem_depth: 0,
        } = self.subtypes[st].constraint
            && self.constrainedness(declared) != Constrainedness::Full
        {
            let (stat, span) = (self.exprs[expr].stat, self.exprs[expr].span);
            let field = self.add_expr(Expr {
                kind: ExprKind::Field { prefix: expr, elem },
                subtype: declared,
                span,
                stat,
                value: None,
            });
            let base = self.base(declared);
            return self.subtypes.push(SubtypeDef {
                base,
                name: None,
                parent: Some(declared),
                constraint: Constraint::FromObject {
                    expr: field,
                    elem_depth: 0,
                },
                resolution: self.subtypes[declared].resolution.clone(),
                span,
            });
        }
        let c = match &self.subtypes[st].constraint {
            Constraint::Record { elems } => elems
                .iter()
                .find(|(i, _)| *i == elem)
                .map(|(_, c)| c.clone()),
            _ => None,
        };
        let Some(c) = c else {
            return declared;
        };
        let base = self.base(declared);
        self.subtypes.push(SubtypeDef {
            base,
            name: None,
            parent: Some(declared),
            constraint: c,
            resolution: self.subtypes[declared].resolution.clone(),
            span: Span::default(),
        })
    }

    /// 5.1: computed, never stored.
    pub fn constrainedness(&self, st: SubtypeId) -> Constrainedness {
        let c = self.subtypes[st].constraint.clone();
        self.constrainedness_of(self.base(st), &c)
    }

    fn constrainedness_of(&self, t: TypeId, c: &Constraint) -> Constrainedness {
        use Constrainedness::*;
        if let Constraint::FromObject { .. } = c {
            return Full;
        }
        match &self.types[t].kind {
            TypeKind::Array { elem, .. } => {
                let (dims, ec) = match c {
                    Constraint::Array { dims, elem: ec } => {
                        (dims.is_some(), self.effective_elem(ec, *elem))
                    }
                    _ => (false, self.subtypes[*elem].constraint.clone()),
                };
                let et = self.base(*elem);
                let composite = matches!(
                    self.types[et].kind,
                    TypeKind::Array { .. } | TypeKind::Record { .. }
                );
                match (dims, self.constrainedness_of(et, &ec)) {
                    (true, Full) => Full,
                    (true, _) => Partial,
                    (false, Full | Partial) if composite => Partial,
                    (false, _) => Unconstrained,
                }
            }
            TypeKind::Record { elems } => {
                let mut any_c = false;
                let mut all_full = true;
                let mut any_composite = false;
                for (i, e) in elems.iter().enumerate() {
                    let ec = match c {
                        Constraint::Record { elems: cs } => cs
                            .iter()
                            .find(|(k, _)| *k as usize == i)
                            .map(|(_, c)| c.clone()),
                        _ => None,
                    }
                    .unwrap_or_else(|| self.subtypes[e.subtype].constraint.clone());
                    let et = self.base(e.subtype);
                    if matches!(
                        self.types[et].kind,
                        TypeKind::Array { .. } | TypeKind::Record { .. }
                    ) {
                        any_composite = true;
                        match self.constrainedness_of(et, &ec) {
                            Full => any_c = true,
                            Partial => {
                                any_c = true;
                                all_full = false;
                            }
                            Unconstrained => all_full = false,
                        }
                    }
                }
                match (all_full, any_c || !any_composite) {
                    (true, _) => Full,
                    (false, true) => Partial,
                    (false, false) => Unconstrained,
                }
            }
            _ => Full,
        }
    }

    pub fn is_poison_expr(&self, e: ExprId) -> bool {
        matches!(self.exprs[e].kind, ExprKind::Error)
    }

    fn register_well_known_type(&mut self, scope: ScopeId, name: Ident, ty: TypeId) {
        if self.well_known.standard != Some(scope) {
            return;
        }
        let w = &mut self.well_known;
        match name.key.as_str() {
            "boolean" => w.boolean = Some(ty),
            "bit" => w.bit = Some(ty),
            "character" => w.character = Some(ty),
            "severity_level" => w.severity_level = Some(ty),
            "integer" => w.integer = Some(ty),
            "real" => w.real = Some(ty),
            "time" => w.time = Some(ty),
            "string" => w.string = Some(ty),
            "file_open_kind" => w.file_open_kind = Some(ty),
            "file_open_status" => w.file_open_status = Some(ty),
            "range_direction" => w.range_direction = Some(ty),
            _ => {}
        }
        self.flush_pending_implicit();
    }

    fn register_well_known_subtype(&mut self, scope: ScopeId, name: Ident, st: SubtypeId) {
        if self.well_known.standard != Some(scope) {
            return;
        }
        let w = &mut self.well_known;
        match name.key.as_str() {
            "natural" => w.natural = Some(st),
            "positive" => w.positive = Some(st),
            "delay_length" => w.delay_length = Some(st),
            _ => {}
        }
        self.flush_pending_implicit();
    }
}

impl SubtypeDef {
    /// A named copy (a subtype declaration names a new subtype of the same constraint).
    fn clone_def(&self, of: SubtypeId, name: Ident, span: Span) -> SubtypeDef {
        SubtypeDef {
            base: self.base,
            name: Some(name),
            parent: Some(of),
            constraint: self.constraint.clone(),
            resolution: self.resolution.clone(),
            span,
        }
    }
}

fn range_span(r: &ast::Range) -> Span {
    match r {
        ast::Range::Simple { left, right, .. } => left.span.to(right.span),
        ast::Range::Name(n) => n.span,
    }
}

/// The `range` keyword before `r`, else `r` itself (the AST keeps no keyword span).
/// `a'range`, `a'reverse_range` or the form with a dimension, `a'range(2)`: a range
/// attribute name, not a type mark.
fn is_attribute_name(n: &ast::Name) -> bool {
    match &n.kind {
        ast::NameKind::Attribute { .. } => true,
        ast::NameKind::Apply { prefix, .. } => {
            matches!(prefix.kind, ast::NameKind::Attribute { .. })
        }
        _ => false,
    }
}

fn range_kw_span(db: &Db, r: &ast::Range) -> Span {
    let sp = range_span(r);
    let b = db.sources.get(sp.file).bytes();
    let end = b[..sp.start as usize].trim_ascii_end().len();
    match end.checked_sub(5) {
        Some(st) if b[st..end].eq_ignore_ascii_case(b"range") => Span {
            start: st as u32,
            end: end as u32,
            ..sp
        },
        _ => sp,
    }
}

fn resolution_span(r: &ast::Resolution) -> Span {
    match r {
        ast::Resolution::Function(n) => n.span,
        ast::Resolution::Element(e) => resolution_span(e),
        ast::Resolution::Record(v) => v.first().map_or(Span::default(), |(i, _)| i.span),
    }
}
