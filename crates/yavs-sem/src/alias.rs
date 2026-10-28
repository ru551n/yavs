//! Aliases and signatures (sem.md 6.2, 6.4; LRM 6.6, 4.5.3).
//!
//! The `Alias` data type lives in `decl.rs`; this module owns alias and
//! signature analysis. The functions here take what name resolution already
//! found (the candidate declarations of the aliased name, the target object
//! expression, the resolved signature type marks), create the alias
//! declarations in the arena and return their ids; the caller makes them
//! visible in its region (`scope.rs`), so redeclaration checks stay in one
//! place.
//!
//! Lookup is transparent: [`Db::entity`] follows nonobject aliases to
//! the entity, and [`Db::describe_decl`] names the alias spelling the user
//! wrote.

use yavs_diag::{Diagnostic, Span};
use yavs_syntax::ast;

use crate::LangRev;
use crate::db::{Db, Reported};
use crate::decl::Body;
use crate::decl::{Alias, Decl, DeclKind, Designator};
use crate::diag;
use crate::hir::{ExprKind, RangeC};
use crate::ids::{DeclId, ExprId, ScopeId, SubtypeId, TypeId};
use crate::implicit::ImplicitOp;
use crate::stat::Staticness;
use crate::types::TypeKind;

/// A resolved signature `[t1, t2 return r]` (4.5.3): the base types of its
/// type marks. `ret` is `Some` exactly when `return` was written.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Signature {
    pub params: Box<[TypeId]>,
    pub ret: Option<TypeId>,
    pub span: Span,
}

impl Signature {
    /// From the subtypes the type marks denote.
    pub fn new(db: &Db, params: &[SubtypeId], ret: Option<SubtypeId>, span: Span) -> Self {
        Signature {
            params: params.iter().map(|&s| db.base(s)).collect(),
            ret: ret.map(|s| db.base(s)),
            span,
        }
    }
}

impl Db {
    /// `` `str_write` (alias of `write`) `` for aliases, `` `write` `` otherwise.
    pub fn describe_decl(&self, d: DeclId) -> String {
        let t = self.entity(d);
        if t == d {
            format!("`{}`", self.decls[d].name)
        } else {
            format!(
                "`{}` (alias of `{}`)",
                self.decls[d].name, self.decls[t].name
            )
        }
    }

    fn same_base(&self, a: TypeId, b: TypeId) -> bool {
        a == b
            || matches!(self.types[a].kind, TypeKind::Error)
            || matches!(self.types[b].kind, TypeKind::Error)
    }

    /// Does `sig` match the profile of `d` (4.5.3)? Parameter base types in
    /// order, and `return` present exactly for functions with the same result
    /// base type.
    pub fn signature_matches(&self, d: DeclId, sig: &Signature) -> bool {
        let Some(p) = self.profile(d) else {
            return false;
        };
        p.params.len() == sig.params.len()
            && p.params
                .iter()
                .zip(sig.params.iter())
                .all(|(&a, &b)| self.same_base(a, b))
            && match (p.ret, sig.ret) {
                (Some(a), Some(b)) => self.same_base(a, b),
                (None, None) => true,
                _ => false,
            }
    }

    /// `[integer, bit return boolean]`
    pub fn display_signature(&self, sig: &Signature) -> String {
        self.display_profile(&sig.params, sig.ret)
    }

    fn display_profile(&self, params: &[TypeId], ret: Option<TypeId>) -> String {
        let mut s = String::from("[");
        let names: Vec<String> = params.iter().map(|&t| self.type_name(t)).collect();
        s.push_str(&names.join(", "));
        if let Some(r) = ret {
            if !params.is_empty() {
                s.push(' ');
            }
            s.push_str("return ");
            s.push_str(&self.type_name(r));
        }
        s.push(']');
        s
    }

    /// Pick the one candidate matching `sig` (6.4): `E1630` for none, `E1631`
    /// for several. Candidates that are aliases of the same entity count once.
    pub fn match_signature(
        &mut self,
        cands: &[DeclId],
        sig: &Signature,
    ) -> Result<DeclId, Reported> {
        if cands.iter().any(|&c| self.is_poison(c)) {
            return Err(Reported);
        }
        let mut hits: Vec<DeclId> = Vec::new();
        for &c in cands {
            if self.signature_matches(c, sig)
                && !hits.iter().any(|&h| self.entity(h) == self.entity(c))
            {
                hits.push(c);
            }
        }
        let name = cands
            .first()
            .map_or_else(String::new, |&c| format!("`{}` ", self.decls[c].name));
        match hits.len() {
            1 => Ok(hits[0]),
            0 => {
                let mut d = Diagnostic::error(
                    diag::SIGNATURE_NONE,
                    format!(
                        "no subprogram or enumeration literal {name}matches signature {}",
                        self.display_signature(sig)
                    ),
                )
                .with_primary(sig.span, "no declaration has this profile");
                d = self.label_candidates(d, cands);
                Err(self.error(d))
            }
            _ => {
                let d = Diagnostic::error(
                    diag::AMBIGUOUS,
                    format!(
                        "signature {} matches more than one declaration of {name}",
                        self.display_signature(sig)
                    ),
                )
                .with_primary(sig.span, "ambiguous signature");
                let d = self.label_candidates(d, &hits);
                Err(self.error(d))
            }
        }
    }

    /// One secondary label per candidate with its profile (8.3: at most 8).
    fn label_candidates(&self, mut d: Diagnostic, cands: &[DeclId]) -> Diagnostic {
        for &c in cands.iter().take(8) {
            let what = match self.profile(c) {
                Some(p) => format!(
                    "{} {}",
                    self.describe_decl(c),
                    self.display_profile(&p.params, p.ret)
                ),
                None => format!(
                    "{} is not a subprogram or enumeration literal",
                    self.describe_decl(c)
                ),
            };
            d = d.with_secondary(self.decls[c].span, what);
        }
        if cands.len() > 8 {
            d = d.with_note(format!("and {} more", cands.len() - 8));
        }
        d
    }

    pub(crate) fn is_poison(&self, d: DeclId) -> bool {
        matches!(self.decls[d].kind, DeclKind::Error)
    }

    /// A nonobject alias (6.6.3). `cands` is what the aliased name (without
    /// its signature) denotes: one declaration, or the overloads of a
    /// subprogram or literal name. Returns the alias first, then the implicit
    /// aliases an alias of a type declares (its literals, units and predefined
    /// operations), all in the arena and not yet declared in `scope`.
    pub fn nonobject_alias(
        &mut self,
        scope: ScopeId,
        ast: &ast::AliasDecl,
        cands: &[DeclId],
        sig: Option<&Signature>,
    ) -> Result<Vec<DeclId>, Reported> {
        let Some(&first) = cands.first() else {
            return Err(self.ice(ast.name.span, "nonobject alias without candidates"));
        };
        if cands.iter().any(|&c| self.is_poison(c)) {
            return Err(Reported);
        }
        if ast.subtype.is_some() {
            // A subtype indication makes this an object alias (6.6.2) of a non-object.
            return Err(self.error(
                Diagnostic::error(
                    diag::ALIAS_NOT_STATIC,
                    format!("{} is not an object", self.describe_decl(first)),
                )
                .with_primary(ast.name.span, "not an object")
                .with_note("LRM 6.6: only an object alias takes a subtype indication"),
            ));
        }
        let target = match sig {
            Some(sig) => self.match_signature(cands, sig)?,
            None => {
                let mut distinct: Vec<DeclId> = Vec::new();
                for &c in cands {
                    if !distinct.iter().any(|&h| self.entity(h) == self.entity(c)) {
                        distinct.push(c);
                    }
                }
                if distinct.len() > 1 {
                    let d = Diagnostic::error(
                        diag::ALIAS_SIGNATURE,
                        format!(
                            "`{}` is overloaded; the alias needs a signature",
                            self.decls[first].name
                        ),
                    )
                    .with_primary(ast.name.span, "names several declarations")
                    .with_help("add a signature, e.g. `alias a is f [integer return bit];`");
                    let d = self.label_candidates(d, &distinct);
                    return Err(self.error(d));
                }
                first
            }
        };
        let name = designator(&ast.designator);
        self.check_alias_designator(&name, target, ast)?;
        let alias = self.decls.push(Decl {
            name,
            kind: DeclKind::Alias(Alias::NonObject { target }),
            scope,
            span: ast.span,
            implicit: false,
            since: LangRev::Vhdl1993,
        });
        let mut out = vec![alias];
        let base = match self.decls[self.entity(target)].kind {
            DeclKind::Type(t) | DeclKind::GenericType(t) => Some(t),
            DeclKind::Subtype(s) => Some(self.base(s)),
            _ => None,
        };
        if let Some(t) = base {
            for d in self.type_implicit_entities(t) {
                let n = self.decls[d].name;
                out.push(self.decls.push(Decl {
                    name: n,
                    kind: DeclKind::Alias(Alias::NonObject { target: d }),
                    scope,
                    span: ast.span,
                    implicit: true,
                    since: self.decls[d].since,
                }));
            }
        }
        Ok(out)
    }

    /// Literals, units and predefined operations of type `t`, in declaration
    /// order: what an alias of `t` implicitly aliases (6.6.3 b to d).
    fn type_implicit_entities(&self, t: TypeId) -> Vec<DeclId> {
        let mut out: Vec<DeclId> = match &self.types[t].kind {
            TypeKind::Enum { lits } => lits.to_vec(),
            TypeKind::Physical { units } => units.iter().map(|u| u.decl).collect(),
            _ => Vec::new(),
        };
        let region = self.decls[self.types[t].decl].scope;
        for &d in &self.scopes[region].order {
            let decl = &self.decls[d];
            if let DeclKind::Subprogram(s) = &decl.kind
                && decl.implicit
                && matches!(s.body, Body::Implicit(ImplicitOp { ty, .. }) if ty == t)
            {
                out.push(d);
            }
        }
        out
    }

    fn check_alias_designator(
        &mut self,
        name: &Designator,
        target: DeclId,
        ast: &ast::AliasDecl,
    ) -> Result<(), Reported> {
        let t = self.entity(target);
        let bad = |db: &mut Db, msg: String, label: &str| {
            Err(db.error(
                Diagnostic::error(diag::BAD_ALIAS, msg)
                    .with_primary(ast.span, label.to_string())
                    .with_secondary(db.decls[t].span, "declared here")
                    .with_note("LRM 6.6.3"),
            ))
        };
        match &self.decls[t].kind {
            DeclKind::Label(_) => {
                return bad(
                    self,
                    format!("label {} cannot be aliased", self.describe_decl(t)),
                    "alias of a label",
                );
            }
            DeclKind::Object(_) | DeclKind::Alias(Alias::Object { .. }) => {
                return Err(self.ice(ast.span, "object passed to nonobject_alias"));
            }
            _ => {}
        }
        match name {
            Designator::Char(c) if !matches!(self.decls[t].kind, DeclKind::EnumLit { .. }) => bad(
                self,
                format!(
                    "alias '{c}' must name an enumeration literal, not {}",
                    self.describe_decl(t)
                ),
                "character designator",
            ),
            Designator::Operator { text, key } => {
                let Some(p) = self.profile(t).filter(|p| p.ret.is_some()) else {
                    return bad(
                        self,
                        format!(
                            "operator alias \"{text}\" must name a function, not {}",
                            self.describe_decl(t)
                        ),
                        "operator designator",
                    );
                };
                let n = p.params.len();
                if operator_arity_ok(key.as_str(), n) {
                    Ok(())
                } else {
                    bad(
                        self,
                        format!("operator \"{text}\" cannot take {n} operand(s)"),
                        "wrong number of operands for this operator",
                    )
                }
            }
            _ => Ok(()),
        }
    }

    /// An object alias (6.6.2) of `target`, which must be a static name of an
    /// object; `subtype` is the resolved subtype indication, if written.
    pub fn object_alias(
        &mut self,
        scope: ScopeId,
        ast: &ast::AliasDecl,
        target: ExprId,
        subtype: Option<SubtypeId>,
    ) -> Result<DeclId, Reported> {
        if matches!(self.exprs[target].kind, ExprKind::Error) {
            return Err(Reported);
        }
        let ast::Designator::Ident(id) = &ast.designator else {
            return Err(self.error(
                Diagnostic::error(
                    diag::BAD_ALIAS,
                    "an object alias must be named by an identifier",
                )
                .with_primary(ast.span, "operator or character designator")
                .with_note("LRM 6.6.2"),
            ));
        };
        if !self.is_static_object_name(target) {
            return Err(self.error(
                Diagnostic::error(diag::ALIAS_NOT_STATIC, "the aliased name is not a static name of an object")
                    .with_primary(self.exprs[target].span, "not a static name")
                    .with_note("LRM 6.6.2, 8.1: index and slice expressions of the name must be globally static, and no part may be a function call or `.all`"),
            ));
        }
        // 1993 4.3.3.1: the type of an object alias is not a multidimensional array type (#814).
        if self.file_rev() == LangRev::Vhdl1993
            && let crate::types::TypeKind::Array { indexes, .. } =
                &self.types[self.base(self.exprs[target].subtype)].kind
            && indexes.len() > 1
        {
            return Err(self.error(
                Diagnostic::error(
                    diag::NOT_IN_REVISION,
                    "an alias of a multidimensional array object is not allowed in VHDL-1993",
                )
                .with_primary(self.exprs[target].span, "a multidimensional array")
                .with_note("LRM 1993 4.3.3.1: the type of an object alias must not be a multidimensional array type (VHDL-2008 allows it)"),
            ));
        }
        if let Some(st) = subtype {
            if matches!(self.exprs[target].kind, ExprKind::External(_)) {
                return Err(self.error(
                    Diagnostic::error(
                        diag::BAD_ALIAS,
                        "an alias of an external name cannot have a subtype indication",
                    )
                    .with_primary(
                        ast.subtype.as_ref().map_or(ast.span, |s| s.span),
                        "remove this",
                    )
                    .with_note("LRM 6.6.2: the external name already gives the subtype"),
                ));
            }
            let (want, have) = (self.base(st), self.base(self.exprs[target].subtype));
            if !self.same_base(want, have) {
                return Err(self.error(
                    Diagnostic::error(
                        diag::TYPE_MISMATCH,
                        format!(
                            "alias subtype is of type `{}`, but the aliased object is of type `{}`",
                            self.type_name(want),
                            self.type_name(have)
                        ),
                    )
                    // The written indication: a bare type mark's subtype span is its declaration.
                    .with_primary(
                        ast.subtype.as_ref().map_or(ast.span, |s| s.span),
                        format!("type `{}`", self.type_name(want)),
                    )
                    .with_secondary(
                        self.exprs[target].span,
                        format!("type `{}`", self.type_name(have)),
                    )
                    .with_note("LRM 6.6.2: the base types must be the same"),
                ));
            }
            // 6.6.2: each element of the object has a matching element of the alias subtype.
            let have_st = self.exprs[target].subtype;
            let dims = match &self.types[want].kind {
                crate::types::TypeKind::Array { indexes, .. } => indexes.len(),
                _ => 0,
            };
            let sr = crate::expr::aggregate::subtype_range;
            if let Some((w, h)) = (0..dims).find_map(|k| {
                let (w, h) = (sr(self, st, k)?.len(), sr(self, have_st, k)?.len());
                (w != h).then_some((w, h))
            }) {
                return Err(self.error(
                    Diagnostic::error(
                        diag::LENGTH_MISMATCH,
                        format!("the alias subtype has {w} elements, the aliased object {h}"),
                    )
                    .with_primary(
                        ast.subtype.as_ref().map_or(ast.span, |s| s.span),
                        format!("{w} elements"),
                    )
                    .with_secondary(self.exprs[target].span, format!("{h} elements"))
                    .with_note(
                        "LRM 6.6.2: each element of the object needs a matching element of the alias subtype",
                    ),
                ));
            }
        }
        // An alias does not read its target (a read through the alias is not tracked, L138).
        self.not_read(target);
        Ok(self.decls.push(Decl {
            name: Designator::Ident(*id),
            kind: DeclKind::Alias(Alias::Object { target, subtype }),
            scope,
            span: ast.span,
            implicit: false,
            since: LangRev::Vhdl1993,
        }))
    }

    /// A static name (8.1) denoting an object: object, object alias or external
    /// name, through fields, and indexes and slices whose expressions are
    /// globally static. A dereference or function call breaks it.
    pub fn is_static_object_name(&self, e: ExprId) -> bool {
        let stat = |x: ExprId| self.exprs[x].stat >= Staticness::Globally;
        match &self.exprs[e].kind {
            ExprKind::Obj(d) => match &self.decls[*d].kind {
                DeclKind::Object(_) | DeclKind::Error => true,
                DeclKind::Alias(Alias::Object { target, .. }) => {
                    self.is_static_object_name(*target)
                }
                _ => false,
            },
            ExprKind::External(_) | ExprKind::Error => true,
            ExprKind::Field { prefix, .. } => self.is_static_object_name(*prefix),
            ExprKind::Index { prefix, indices } => {
                self.is_static_object_name(*prefix) && indices.iter().all(|&i| stat(i))
            }
            ExprKind::Slice { prefix, range } => {
                self.is_static_object_name(*prefix)
                    && match range {
                        RangeC::Explicit { left, right, .. } => stat(*left) && stat(*right),
                        // `a'range` of a static name, or of a type mark: static as the prefix is.
                        RangeC::Attr { prefix: p, .. } => match p {
                            crate::hir::AttrPrefix::Expr(x) => self.is_static_object_name(*x),
                            _ => true,
                        },
                    }
            }
            _ => false,
        }
    }
}

/// The designator of an alias declaration, original spelling kept.
pub fn designator(d: &ast::Designator) -> Designator {
    match d {
        ast::Designator::Ident(i) => Designator::Ident(*i),
        ast::Designator::Operator(s, _) => Designator::operator(s.as_str()),
        ast::Designator::Char(c) => Designator::Char(c.value),
    }
}

/// Number of operands an operator symbol accepts (9.2; 2008 unary logical).
/// `key` is the folded key with quotes.
fn operator_arity_ok(key: &str, n: usize) -> bool {
    match key.trim_matches('"') {
        "and" | "or" | "nand" | "nor" | "xor" | "xnor" | "+" | "-" => n == 1 || n == 2,
        "not" | "abs" | "??" => n == 1,
        _ => n == 2,
    }
}

#[cfg(test)]
pub(crate) mod fixture {
    //! Hand-built entities for unit tests of S6/S7 modules, until whole-file
    //! analysis exists (S1).

    use yavs_diag::Span;
    use yavs_syntax::Symbol;
    use yavs_syntax::ast::Ident;

    use crate::db::Db;
    use crate::decl::*;
    use crate::hir::{Expr, ExprKind, Mode};
    use crate::ids::*;
    use crate::implicit::{Family, ImplicitOp};
    use crate::scope::ScopeKind;
    use crate::stat::Staticness;
    use crate::types::*;
    use crate::{LangRev, Options};

    pub fn db() -> Db {
        db_rev(LangRev::Vhdl2008)
    }

    pub fn db_rev(rev: LangRev) -> Db {
        Db::new(Options::new(rev, crate::default_stdlib()))
    }

    /// A span that is distinct per call site value, so labels can be told apart.
    pub fn sp(n: u32) -> Span {
        Span::new(yavs_diag::FileId(0), n, n + 1)
    }

    pub fn ident(s: &str) -> Ident {
        Ident {
            text: Symbol::intern(s),
            key: Symbol::intern(&s.to_ascii_lowercase()),
            span: Span::default(),
        }
    }

    pub fn region(db: &mut Db, kind: ScopeKind) -> ScopeId {
        let root = db.root;
        db.add_scope(kind, Some(root), None)
    }

    pub fn declare(db: &mut Db, scope: ScopeId, name: Designator, kind: DeclKind) -> DeclId {
        let d = db.decls.push(Decl {
            name,
            kind,
            scope,
            span: sp(db.decls.len() as u32),
            implicit: false,
            since: LangRev::Vhdl1993,
        });
        db.scopes[scope]
            .names
            .entry(name.key())
            .or_default()
            .push(d);
        db.scopes[scope].order.push(d);
        d
    }

    /// A type with its first subtype and declaration.
    pub fn new_type(
        db: &mut Db,
        scope: ScopeId,
        name: &str,
        kind: TypeKind,
    ) -> (TypeId, SubtypeId) {
        let ty = db.types.next_id();
        let st = db.subtypes.push(SubtypeDef {
            base: ty,
            name: Some(ident(name)),
            parent: None,
            constraint: Constraint::None,
            resolution: None,
            span: sp(1000 + ty_index(ty)),
        });
        let d = declare(
            db,
            scope,
            Designator::Ident(ident(name)),
            DeclKind::Type(ty),
        );
        db.types.push(TypeDef {
            decl: d,
            name: ident(name),
            anonymous: false,
            kind,
            first_subtype: st,
            base_subtype: st,
        });
        (ty, st)
    }

    fn ty_index(t: TypeId) -> u32 {
        use crate::ids::Id;
        t.index() as u32
    }

    pub fn int_type(db: &mut Db, scope: ScopeId, name: &str) -> (TypeId, SubtypeId) {
        new_type(db, scope, name, TypeKind::Integer)
    }

    /// An enumeration type; literals starting with `'` are character literals.
    pub fn enum_type(
        db: &mut Db,
        scope: ScopeId,
        name: &str,
        lits: &[&str],
    ) -> (TypeId, SubtypeId) {
        let (ty, st) = new_type(db, scope, name, TypeKind::Enum { lits: Box::new([]) });
        let ds: Vec<DeclId> = lits
            .iter()
            .enumerate()
            .map(|(i, l)| {
                let n = match l.strip_prefix('\'') {
                    Some(c) => Designator::Char(c.chars().next().unwrap()),
                    None => Designator::Ident(ident(l)),
                };
                declare(db, scope, n, DeclKind::EnumLit { ty, pos: i as u32 })
            })
            .collect();
        db.types[ty].kind = TypeKind::Enum { lits: ds.into() };
        (ty, st)
    }

    pub fn record_type(
        db: &mut Db,
        scope: ScopeId,
        name: &str,
        elems: &[(&str, SubtypeId)],
    ) -> (TypeId, SubtypeId) {
        let elems = elems
            .iter()
            .map(|(n, s)| RecordElem {
                name: ident(n),
                subtype: *s,
            })
            .collect();
        new_type(db, scope, name, TypeKind::Record { elems })
    }

    pub fn array_type(
        db: &mut Db,
        scope: ScopeId,
        name: &str,
        index: SubtypeId,
        elem: SubtypeId,
    ) -> (TypeId, SubtypeId) {
        new_type(
            db,
            scope,
            name,
            TypeKind::Array {
                indexes: Box::new([index]),
                elem,
            },
        )
    }

    pub fn protected_type(db: &mut Db, scope: ScopeId, name: &str) -> (TypeId, SubtypeId, ScopeId) {
        let ps = region(db, ScopeKind::Protected);
        db.scopes[ps].parent = Some(scope);
        let (t, s) = new_type(
            db,
            scope,
            name,
            TypeKind::Protected {
                decl_scope: ps,
                body: None,
                generics: None,
            },
        );
        db.scopes[ps].owner = Some(db.types[t].decl);
        (t, s, ps)
    }

    fn param(db: &Db, st: SubtypeId, i: usize) -> Param {
        let _ = db;
        Param {
            name: ident(&format!("p{i}")),
            class: ObjClass::Constant,
            mode: Mode::In,
            subtype: st,
            default: None,
            decl: None,
            mark: None,
        }
    }

    pub fn subprogram(
        db: &mut Db,
        scope: ScopeId,
        name: Designator,
        params: &[SubtypeId],
        ret: Option<SubtypeId>,
        body: Body,
    ) -> DeclId {
        let params = params
            .iter()
            .enumerate()
            .map(|(i, &s)| param(db, s, i))
            .collect();
        let kind = match ret {
            Some(_) => SubprogramKind::Function { pure: true },
            None => SubprogramKind::Procedure,
        };
        declare(
            db,
            scope,
            name,
            DeclKind::Subprogram(Subprogram {
                kind,
                params,
                ret,
                generics: None,
                scope: None,
                body,
                mangled: Symbol::intern(""),
                ret_mark: None,
                impure_reads: false,
            }),
        )
    }

    pub fn function(
        db: &mut Db,
        scope: ScopeId,
        name: &str,
        params: &[SubtypeId],
        ret: Option<SubtypeId>,
    ) -> DeclId {
        subprogram(
            db,
            scope,
            Designator::Ident(ident(name)),
            params,
            ret,
            Body::Pending,
        )
    }

    pub fn implicit_op(
        db: &mut Db,
        scope: ScopeId,
        op: &str,
        family: Family,
        ty: TypeId,
        params: &[SubtypeId],
        ret: SubtypeId,
    ) -> DeclId {
        let d = subprogram(
            db,
            scope,
            Designator::operator(op),
            params,
            Some(ret),
            Body::Implicit(ImplicitOp {
                family,
                ty,
                form: crate::implicit::Form::Plain,
            }),
        );
        db.decls[d].implicit = true;
        d
    }

    pub fn object(
        db: &mut Db,
        scope: ScopeId,
        name: &str,
        class: ObjClass,
        st: SubtypeId,
    ) -> (DeclId, ExprId) {
        let d = declare(
            db,
            scope,
            Designator::Ident(ident(name)),
            DeclKind::Object(Object {
                class,
                role: ObjRole::Plain,
                mode: None,
                subtype: st,
                default: None,
                bounds: BoundsFrom::Subtype,
                signal_kind: None,
                file_open: None,
            }),
        );
        let e = expr(db, ExprKind::Obj(d), st, Staticness::None);
        (d, e)
    }

    pub fn expr(db: &mut Db, kind: ExprKind, st: SubtypeId, stat: Staticness) -> ExprId {
        let span = sp(5000 + db.exprs.len() as u32);
        db.add_expr(Expr {
            kind,
            subtype: st,
            span,
            stat,
            value: None,
        })
    }

    /// The declarations of the first package in `src`.
    pub fn parse_decls(src: &str, std: yavs_syntax::Std) -> Vec<yavs_syntax::ast::Decl> {
        use yavs_syntax::ast;
        let opts = yavs_syntax::Options {
            std,
            defines: Vec::new(),
            psl: false,
        };
        let mut sm = yavs_diag::SourceMap::default();
        let f = sm.add("t.vhd", src);
        let parsed = yavs_syntax::parse_file(f, sm.get(f), &opts);
        assert!(parsed.diags.is_empty(), "{:?}", parsed.diags);
        let unit = parsed
            .design
            .expect("parsed")
            .units
            .into_iter()
            .next()
            .expect("one unit");
        let ast::LibraryUnit::Package(p) = unit.unit else {
            panic!("not a package")
        };
        p.decls
    }

    /// Codes of all diagnostics reported so far.
    pub fn codes(db: &Db) -> Vec<&'static str> {
        db.diagnostics().iter().map(|d| d.code).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::*;
    use super::*;
    use crate::decl::ObjClass;
    use crate::implicit::Family;
    use crate::scope::ScopeKind;
    use yavs_syntax::Symbol;

    fn alias_ast(name: ast::Designator, subtype: Option<ast::SubtypeIndication>) -> ast::AliasDecl {
        ast::AliasDecl {
            designator: name,
            subtype,
            name: ast::Name {
                kind: ast::NameKind::Simple(ident("target")),
                span: sp(900),
            },
            span: sp(901),
        }
    }

    fn id(s: &str) -> ast::Designator {
        ast::Designator::Ident(ident(s))
    }

    fn st_ind() -> ast::SubtypeIndication {
        ast::SubtypeIndication {
            resolution: None,
            type_mark: ast::Name {
                kind: ast::NameKind::Simple(ident("t")),
                span: sp(902),
            },
            constraint: None,
            span: sp(902),
        }
    }

    struct Pkg {
        db: Db,
        s: ScopeId,
        int: SubtypeId,
        bit: SubtypeId,
    }

    fn pkg() -> Pkg {
        let mut db = db();
        let s = region(&mut db, ScopeKind::Package);
        let (_, int) = int_type(&mut db, s, "integer");
        let (_, bit) = enum_type(&mut db, s, "bit", &["'0", "'1"]);
        Pkg { db, s, int, bit }
    }

    #[test]
    fn signature_selects_one_overload() {
        let Pkg {
            mut db,
            s,
            int,
            bit,
        } = pkg();
        let f1 = function(&mut db, s, "h", &[int], Some(int));
        let f2 = function(&mut db, s, "h", &[bit], Some(int));
        let sig = Signature::new(&db, &[bit], Some(int), sp(1));
        assert_eq!(db.match_signature(&[f1, f2], &sig), Ok(f2));
        // A procedure needs a signature without `return`.
        let p = function(&mut db, s, "pr", &[int], None);
        let sig = Signature::new(&db, &[int], Some(int), sp(2));
        assert!(db.match_signature(&[p], &sig).is_err());
        let sig = Signature::new(&db, &[int], None, sp(3));
        assert_eq!(db.match_signature(&[p], &sig), Ok(p));
        assert_eq!(codes(&db), [diag::SIGNATURE_NONE]);
    }

    #[test]
    fn signature_none_and_many() {
        let Pkg {
            mut db,
            s,
            int,
            bit,
        } = pkg();
        let f1 = function(&mut db, s, "h", &[int], Some(int));
        let sig = Signature::new(&db, &[bit, bit], Some(int), sp(1));
        assert!(db.match_signature(&[f1], &sig).is_err());
        // Two homographs (from different packages, say) match the same signature.
        let other = region(&mut db, ScopeKind::Package);
        let f2 = function(&mut db, other, "h", &[int], Some(int));
        let sig = Signature::new(&db, &[int], Some(int), sp(2));
        assert!(db.match_signature(&[f1, f2], &sig).is_err());
        // An alias of f1 is the same entity as f1: not ambiguous.
        let a = declare(
            &mut db,
            other,
            Designator::Ident(ident("g")),
            DeclKind::Alias(Alias::NonObject { target: f1 }),
        );
        assert_eq!(db.match_signature(&[f1, a], &sig), Ok(f1));
        assert_eq!(codes(&db), [diag::SIGNATURE_NONE, diag::AMBIGUOUS]);
        assert_eq!(db.describe_decl(a), "`g` (alias of `h`)");
    }

    #[test]
    fn enum_literal_signature_and_non_overloadable() {
        let Pkg {
            mut db,
            s,
            int,
            bit,
        } = pkg();
        let lit0 = db.scopes[s].names[&Symbol::intern("'0'")][0];
        let sig = Signature::new(&db, &[], Some(bit), sp(1));
        assert_eq!(db.match_signature(&[lit0], &sig), Ok(lit0));
        // `alias kk is k [return integer]` for a constant k: E1630.
        let (k, _) = object(&mut db, s, "k", ObjClass::Constant, int);
        let sig = Signature::new(&db, &[], Some(int), sp(2));
        assert!(db.match_signature(&[k], &sig).is_err());
        assert_eq!(codes(&db), [diag::SIGNATURE_NONE]);
    }

    #[test]
    fn alias_of_overloaded_name_needs_signature() {
        let Pkg {
            mut db,
            s,
            int,
            bit,
        } = pkg();
        let f1 = function(&mut db, s, "h", &[int], Some(int));
        let f2 = function(&mut db, s, "h", &[bit], Some(int));
        assert!(
            db.nonobject_alias(s, &alias_ast(id("hh"), None), &[f1, f2], None)
                .is_err()
        );
        assert_eq!(codes(&db), [diag::ALIAS_SIGNATURE]);
        // Not overloaded: no signature needed (2008 6.6.3, sim1 agrees).
        let g = db
            .nonobject_alias(s, &alias_ast(id("g"), None), &[f1], None)
            .unwrap();
        assert_eq!(g.len(), 1);
        assert_eq!(db.entity(g[0]), f1);
    }

    #[test]
    fn alias_designator_rules() {
        let Pkg {
            mut db,
            s,
            int,
            bit,
        } = pkg();
        let f = function(&mut db, s, "f", &[int], Some(int));
        let p = function(&mut db, s, "pr", &[int], None);
        let unary = ast::Designator::Operator(Symbol::intern("+"), sp(3));
        assert!(
            db.nonobject_alias(s, &alias_ast(unary, None), &[f], None)
                .is_ok()
        );
        let red = ast::Designator::Operator(Symbol::intern("AND"), sp(3));
        assert!(
            db.nonobject_alias(s, &alias_ast(red, None), &[f], None)
                .is_ok()
        );
        let eq = ast::Designator::Operator(Symbol::intern("="), sp(3));
        assert!(
            db.nonobject_alias(s, &alias_ast(eq, None), &[f], None)
                .is_err()
        );
        let plus = ast::Designator::Operator(Symbol::intern("+"), sp(3));
        assert!(
            db.nonobject_alias(s, &alias_ast(plus, None), &[p], None)
                .is_err()
        );
        let ch = ast::Designator::Char(ast::CharLit {
            value: 'x',
            span: sp(4),
        });
        assert!(
            db.nonobject_alias(s, &alias_ast(ch, None), &[f], None)
                .is_err()
        );
        let lit1 = db.scopes[s].names[&Symbol::intern("'1'")][0];
        let ch = ast::Designator::Char(ast::CharLit {
            value: 'h',
            span: sp(4),
        });
        assert!(
            db.nonobject_alias(s, &alias_ast(ch, None), &[lit1], None)
                .is_ok()
        );
        // A subtype indication on a nonobject alias.
        assert!(
            db.nonobject_alias(s, &alias_ast(id("x"), Some(st_ind())), &[f], None)
                .is_err()
        );
        let _ = bit;
        assert_eq!(
            codes(&db),
            [
                diag::BAD_ALIAS,
                diag::BAD_ALIAS,
                diag::BAD_ALIAS,
                diag::ALIAS_NOT_STATIC
            ]
        );
    }

    #[test]
    fn alias_of_type_aliases_literals_and_predefined_ops() {
        let Pkg { mut db, s, int, .. } = pkg();
        let (state, state_st) = enum_type(&mut db, s, "state", &["idle", "run"]);
        let bool_st = int; // the result type does not matter here
        let eq = implicit_op(
            &mut db,
            s,
            "=",
            Family::Eq,
            state,
            &[state_st, state_st],
            bool_st,
        );
        // A user function on the type is not aliased.
        let _user = function(&mut db, s, "f", &[state_st], Some(state_st));
        let tdecl = db.types[state].decl;
        let out = db
            .nonobject_alias(s, &alias_ast(id("st"), None), &[tdecl], None)
            .unwrap();
        let targets: Vec<DeclId> = out.iter().map(|&d| db.entity(d)).collect();
        let idle = db.scopes[s].names[&Symbol::intern("idle")][0];
        let run = db.scopes[s].names[&Symbol::intern("run")][0];
        assert_eq!(targets, [tdecl, idle, run, eq]);
        assert!(out[1..].iter().all(|&d| db.decls[d].implicit));
        assert_eq!(db.decls[out[3]].name.key(), Symbol::intern("\"=\""));
        assert!(db.diagnostics().is_empty());
    }

    #[test]
    fn object_alias_static_name_and_subtype() {
        let Pkg {
            mut db,
            s,
            int,
            bit,
        } = pkg();
        let (_, arr) = array_type(&mut db, s, "bv", int, bit);
        let (_, v) = object(&mut db, s, "v", ObjClass::Signal, arr);
        let (_, i) = object(&mut db, s, "i", ObjClass::Variable, int);
        let lit = expr(&mut db, ExprKind::Error, int, Staticness::Locally);
        db.exprs[lit].kind = ExprKind::Null; // any locally static value
        let idx_static = expr(
            &mut db,
            ExprKind::Index {
                prefix: v,
                indices: Box::new([lit]),
            },
            bit,
            Staticness::None,
        );
        let idx_dyn = expr(
            &mut db,
            ExprKind::Index {
                prefix: v,
                indices: Box::new([i]),
            },
            bit,
            Staticness::None,
        );
        assert!(
            db.object_alias(s, &alias_ast(id("a"), None), idx_static, None)
                .is_ok()
        );
        assert!(
            db.object_alias(s, &alias_ast(id("b"), None), idx_dyn, None)
                .is_err()
        );
        assert!(
            db.object_alias(s, &alias_ast(id("c"), Some(st_ind())), v, Some(int))
                .is_err()
        );
        assert!(
            db.object_alias(s, &alias_ast(id("d"), Some(st_ind())), v, Some(arr))
                .is_ok()
        );
        let op = ast::Designator::Operator(Symbol::intern("+"), sp(1));
        assert!(db.object_alias(s, &alias_ast(op, None), v, None).is_err());
        assert_eq!(
            codes(&db),
            [diag::ALIAS_NOT_STATIC, diag::TYPE_MISMATCH, diag::BAD_ALIAS]
        );
    }
}
