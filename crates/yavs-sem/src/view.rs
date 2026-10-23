//! 2019 mode views and `'converse` (sem.md 6.7, LRM 2019 6.5.2).
//!
//! A view gives every element of a record a mode, or a nested view. Ports
//! `p : view v` and `p : view (v) of arr_t` carry the view in their
//! [`Mode`]; [`Db::port_mode_tree`] expands it into the per-element mode tree
//! that the driver checks here, `lsp.rs` and M2's port splitting (ir.md H10)
//! use. `'converse` is materialized as its own [`ViewDef`] on first use.

use yavs_diag::{Diagnostic, Span};
use yavs_syntax::ast;

use crate::LangRev;
use crate::db::{Db, Reported};
use crate::decl::{Decl, DeclKind};
use crate::diag;
use crate::hir::{ExprKind, LspStep};
use crate::ids::{DeclId, ExprId, ScopeId, SubtypeId, TypeId, ViewId};
use crate::types::TypeKind;

pub use crate::hir::Mode;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct ViewDef {
    pub decl: DeclId,
    pub record: SubtypeId,
    /// One per record element, in element order.
    pub elems: Box<[ElemMode]>,
    /// The converse view, once materialized (both directions are linked).
    pub converse: Option<ViewId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ElemMode {
    Simple(Mode),
    View {
        view: ViewId,
        converse: bool,
        array: bool,
    },
}

/// A resolved mode view indication: `v` or `v'converse`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewRef {
    pub view: ViewId,
    pub converse: bool,
}

/// The effective mode of every sub-element of a port.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModeTree {
    /// A simple mode (never `Mode::View`).
    Leaf(Mode),
    /// One subtree per element of record type `ty`.
    Record { ty: TypeId, elems: Box<[ModeTree]> },
    /// Every element of an array (`view (v) of arr_t`).
    Array(Box<ModeTree>),
}

/// A step from a port to one of its leaves, for M2's port splitting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewStep {
    Field(u32),
    /// All elements of an array.
    Elements,
}

impl ModeTree {
    /// Every leaf with its path, in element order.
    pub fn leaves(&self) -> Vec<(Vec<ViewStep>, Mode)> {
        fn walk(t: &ModeTree, path: &mut Vec<ViewStep>, out: &mut Vec<(Vec<ViewStep>, Mode)>) {
            match t {
                ModeTree::Leaf(m) => out.push((path.clone(), *m)),
                ModeTree::Record { elems, .. } => {
                    for (i, e) in elems.iter().enumerate() {
                        path.push(ViewStep::Field(i as u32));
                        walk(e, path, out);
                        path.pop();
                    }
                }
                ModeTree::Array(e) => {
                    path.push(ViewStep::Elements);
                    walk(e, path, out);
                    path.pop();
                }
            }
        }
        let mut out = Vec::new();
        walk(self, &mut Vec::new(), &mut out);
        out
    }

    /// The mode of the leaf at field path `path` (array levels are passed through), `None`
    /// when the path leaves the tree.
    pub fn leaf_mode(&self, path: &[u32]) -> Option<Mode> {
        let mut t = self;
        let mut p = path;
        loop {
            match t {
                ModeTree::Leaf(m) => return Some(*m),
                ModeTree::Array(e) => t = e,
                ModeTree::Record { elems, .. } => {
                    let (&f, rest) = p.split_first()?;
                    t = elems.get(f as usize)?;
                    p = rest;
                }
            }
        }
    }

    /// The subtree a longest static prefix path selects. Index and slice
    /// steps select into an array tree; on a leaf every step stays there.
    pub fn select(&self, path: &[LspStep]) -> &ModeTree {
        let mut t = self;
        for step in path {
            t = match (t, step) {
                (ModeTree::Record { elems, .. }, LspStep::Field(i)) => match elems.get(*i as usize)
                {
                    Some(e) => e,
                    None => return t,
                },
                (ModeTree::Array(e), LspStep::Index(_)) => e,
                (ModeTree::Array(_), LspStep::Slice(_)) => t,
                _ => t,
            };
        }
        t
    }
}

/// `in` and `out` swap; `inout` and `buffer` stay (user ruling U-4).
pub fn converse_mode(m: Mode) -> Mode {
    match m {
        Mode::In => Mode::Out,
        Mode::Out => Mode::In,
        other => other,
    }
}

impl Db {
    /// The view a resolved name denotes (`E1902` if it is not a view).
    pub fn view_ref(&mut self, d: DeclId, converse: bool, span: Span) -> Result<ViewRef, Reported> {
        let d = self.entity(d);
        match self.decls[d].kind {
            DeclKind::View(view) => Ok(ViewRef { view, converse }),
            DeclKind::Error => Err(Reported),
            _ => Err(self.error(
                Diagnostic::error(
                    diag::VIEW_NOT_RECORD,
                    format!("`{}` is not a mode view", self.decls[d].name),
                )
                .with_primary(span, "expected a mode view")
                .with_secondary(self.decls[d].span, "declared here"),
            )),
        }
    }

    /// A mode view declaration (2019 6.5.2). `record` is its resolved subtype
    /// indication; `resolve` resolves the view names of nested element views
    /// (through [`Db::view_ref`]). Returns the view's declaration, in the arena
    /// and not yet declared in `scope`.
    pub fn declare_view(
        &mut self,
        scope: ScopeId,
        ast: &ast::ViewDecl,
        record: SubtypeId,
        resolve: &mut dyn FnMut(&mut Db, &ast::Name) -> Result<ViewRef, Reported>,
    ) -> Result<DeclId, Reported> {
        let rty = self.base(record);
        let names: Vec<(yavs_syntax::Symbol, SubtypeId)> = match &self.types[rty].kind {
            TypeKind::Record { elems } => elems.iter().map(|e| (e.name.key, e.subtype)).collect(),
            TypeKind::Error => return Err(Reported),
            _ => {
                return Err(self.error(
                    Diagnostic::error(
                        diag::VIEW_NOT_RECORD,
                        format!("`{}` is not a record type", self.type_name(rty)),
                    )
                    .with_primary(ast.subtype.span, "a mode view needs a record subtype")
                    .with_note("LRM 2019 6.5.2"),
                ));
            }
        };
        let rname = self.type_name(rty);
        let vname = ast.name.text;
        let mut elems: Vec<Option<ElemMode>> = vec![None; names.len()];
        let mut res = Ok(());
        for el in &ast.elements {
            let mode = match self.view_elem_mode(&el.mode, el.span, resolve) {
                Ok(m) => Some(m),
                Err(r) => {
                    res = Err(r);
                    None
                }
            };
            for n in &el.names {
                let Some(i) = names.iter().position(|(k, _)| *k == n.key) else {
                    res = Err(self.error(
                        Diagnostic::error(
                            diag::VIEW_ELEMENT,
                            format!("record `{rname}` has no element `{}`", n.text),
                        )
                        .with_primary(n.span, "not an element"),
                    ));
                    continue;
                };
                if elems[i].is_some() {
                    res = Err(self.error(
                        Diagnostic::error(
                            diag::VIEW_ELEMENT,
                            format!("element `{}` appears twice in view `{vname}`", n.text),
                        )
                        .with_primary(n.span, "second mode for this element"),
                    ));
                    continue;
                }
                let Some(m) = mode else { continue };
                let at = match &el.mode {
                    ast::Mode::View { name, .. } => name.span,
                    _ => n.span,
                };
                if let Err(r) = self.check_nested_view(m, names[i].1, n, at) {
                    res = Err(r);
                }
                elems[i] = Some(m);
            }
        }
        let rec_elems = match &self.types[rty].kind {
            TypeKind::Record { elems } => elems.iter().map(|e| e.name.text).collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        for (i, e) in elems.iter().enumerate() {
            if e.is_none() && res.is_ok() {
                res = Err(self.error(
                    Diagnostic::error(
                        diag::VIEW_ELEMENT,
                        format!(
                            "element `{}` of record `{rname}` has no mode in view `{vname}`",
                            rec_elems[i]
                        ),
                    )
                    .with_primary(ast.name.span, "incomplete view")
                    .with_note(
                        "LRM 2019 6.5.2: every element of the record gets a mode exactly once",
                    ),
                ));
            }
        }
        res?;
        let decl = self.decls.next_id();
        let view = self.views.push(ViewDef {
            decl,
            record,
            elems: elems
                .into_iter()
                .map(|e| e.expect("checked above"))
                .collect(),
            converse: None,
        });
        Ok(self.decls.push(Decl {
            name: crate::decl::Designator::Ident(ast.name),
            kind: DeclKind::View(view),
            scope,
            span: ast.span,
            implicit: false,
            since: LangRev::Vhdl2019,
        }))
    }

    fn view_elem_mode(
        &mut self,
        m: &ast::Mode,
        span: Span,
        resolve: &mut dyn FnMut(&mut Db, &ast::Name) -> Result<ViewRef, Reported>,
    ) -> Result<ElemMode, Reported> {
        Ok(match m {
            ast::Mode::In => ElemMode::Simple(Mode::In),
            ast::Mode::Out => ElemMode::Simple(Mode::Out),
            ast::Mode::Inout => ElemMode::Simple(Mode::Inout),
            ast::Mode::Buffer => ElemMode::Simple(Mode::Buffer),
            ast::Mode::Linkage => {
                return Err(self.error(
                    Diagnostic::error(
                        diag::VIEW_ELEMENT,
                        "an element of a mode view cannot have mode `linkage`",
                    )
                    .with_primary(span, "linkage")
                    .with_note("LRM 2019 6.5.2"),
                ));
            }
            ast::Mode::View { name, array } => {
                let r = resolve(self, name)?;
                ElemMode::View {
                    view: r.view,
                    converse: r.converse,
                    array: *array,
                }
            }
        })
    }

    /// `d : view w` needs `d` of `w`'s record type; `d : view (w)` an array of it.
    fn check_nested_view(
        &mut self,
        m: ElemMode,
        elem: SubtypeId,
        n: &yavs_syntax::ast::Ident,
        at: Span,
    ) -> Result<(), Reported> {
        let ElemMode::View { view, array, .. } = m else {
            return Ok(());
        };
        let want = self.base(self.views[view].record);
        let mut have = self.base(elem);
        if array {
            match &self.types[have].kind {
                TypeKind::Array { elem, .. } => have = self.base(*elem),
                TypeKind::Error => return Ok(()),
                _ => {
                    return Err(self.error(
                        Diagnostic::error(
                            diag::VIEW_ELEMENT,
                            format!(
                                "element `{}` has type `{}`, which is not an array",
                                n.text,
                                self.type_name(have)
                            ),
                        )
                        .with_primary(n.span, "array mode view on a non-array element"),
                    ));
                }
            }
        }
        if want == have || matches!(self.types[have].kind, TypeKind::Error) {
            return Ok(());
        }
        let vname = self.decls[self.views[view].decl].name;
        Err(self.error(
            Diagnostic::error(
                diag::VIEW_ELEMENT,
                format!(
                    "view `{vname}` is for `{}`, element `{}` has type `{}`",
                    self.type_name(want),
                    n.text,
                    self.type_name(have)
                ),
            )
            .with_primary(at, "view of another record type")
            .with_secondary(n.span, "this element"),
        ))
    }

    /// The converse of view `v`, materialized once (2019 6.5.2, 16.2).
    pub fn converse(&mut self, v: ViewId) -> ViewId {
        if let Some(c) = self.views[v].converse {
            return c;
        }
        let elems = self.views[v]
            .elems
            .iter()
            .map(|e| match *e {
                ElemMode::Simple(m) => ElemMode::Simple(converse_mode(m)),
                ElemMode::View {
                    view,
                    converse,
                    array,
                } => ElemMode::View {
                    view,
                    converse: !converse,
                    array,
                },
            })
            .collect();
        let od = self.views[v].decl;
        let decl = self.decls.next_id();
        let c = self.views.push(ViewDef {
            decl,
            record: self.views[v].record,
            elems,
            converse: Some(v),
        });
        let (name, scope, span) = (
            self.decls[od].name,
            self.decls[od].scope,
            self.decls[od].span,
        );
        self.decls.push(Decl {
            name,
            kind: DeclKind::View(c),
            scope,
            span,
            implicit: true,
            since: LangRev::Vhdl2019,
        });
        self.views[v].converse = Some(c);
        c
    }

    /// The view a reference denotes, `'converse` applied.
    pub fn effective_view(&mut self, r: ViewRef) -> ViewId {
        if r.converse {
            self.converse(r.view)
        } else {
            r.view
        }
    }

    /// The mode of an interface object `p : view v [of t]` or
    /// `p : view (v) of arr_t`, and its subtype. The class must be signal
    /// (`E1903`); the subtype must be the view's record (or an array of it).
    pub fn view_port(
        &mut self,
        class: Option<ast::InterfaceClass>,
        r: ViewRef,
        array: bool,
        subtype: Option<SubtypeId>,
        span: Span,
    ) -> Result<(Mode, SubtypeId), Reported> {
        if !matches!(class, None | Some(ast::InterfaceClass::Signal)) {
            return Err(self.error(
                Diagnostic::error(diag::VIEW_MODE, "only a signal can have a mode view")
                    .with_primary(span, "not a signal"),
            ));
        }
        let view = self.effective_view(r);
        let rec = self.views[view].record;
        let want = self.base(rec);
        let st = match (subtype, array) {
            (None, false) => rec,
            (None, true) => return Err(self.ice(span, "array mode view without a subtype")),
            (Some(st), _) => {
                let mut have = self.base(st);
                if array && let TypeKind::Array { elem, .. } = &self.types[have].kind {
                    have = self.base(*elem);
                }
                if have != want && !matches!(self.types[have].kind, TypeKind::Error) {
                    let msg = if array {
                        format!(
                            "`{}` is not an array of `{}`",
                            self.type_name(self.base(st)),
                            self.type_name(want)
                        )
                    } else {
                        format!(
                            "view is for `{}`, the port has type `{}`",
                            self.type_name(want),
                            self.type_name(have)
                        )
                    };
                    return Err(self.error(
                        Diagnostic::error(diag::TYPE_MISMATCH, msg)
                            .with_primary(self.subtypes[st].span, "does not match the view"),
                    ));
                }
                st
            }
        };
        Ok((Mode::View { view, array }, st))
    }

    /// The mode tree of an interface mode (a leaf for simple modes).
    pub fn port_mode_tree(&self, m: Mode) -> ModeTree {
        match m {
            Mode::View { view, array: false } => self.view_tree(view, false),
            Mode::View { view, array: true } => {
                ModeTree::Array(Box::new(self.view_tree(view, false)))
            }
            m => ModeTree::Leaf(m),
        }
    }

    /// The paths below a signal with mode view `m` to the elements it drives (`out`,
    /// `inout`, `buffer`), outermost first. A `Field` step after an array (`view (v) of
    /// arr_t`) names that element of every array element.
    pub fn view_driven_paths(&self, m: Mode) -> Vec<Vec<LspStep>> {
        self.view_driven_below(m, &[])
    }

    /// Like `view_driven_paths`, for the part of the signal that the formal part `steps`
    /// names (an individual association, 6.5.7.1): the paths are relative to that part.
    pub fn view_driven_below(&self, m: Mode, steps: &[LspStep]) -> Vec<Vec<LspStep>> {
        self.view_paths(m, steps, |m| {
            matches!(m, Mode::Out | Mode::Inout | Mode::Buffer)
        })
    }

    /// The paths below a signal with mode view `m` to the elements it reads (`in`, `inout`):
    /// the sensitivity of a concurrent procedure call (11.4).
    pub fn view_read_paths(&self, m: Mode) -> Vec<Vec<LspStep>> {
        self.view_paths(m, &[], |m| matches!(m, Mode::In | Mode::Inout))
    }

    fn view_paths(&self, m: Mode, steps: &[LspStep], keep: fn(Mode) -> bool) -> Vec<Vec<LspStep>> {
        fn walk(
            t: &ModeTree,
            path: &mut Vec<LspStep>,
            out: &mut Vec<Vec<LspStep>>,
            keep: fn(Mode) -> bool,
        ) {
            match t {
                ModeTree::Leaf(m) if keep(*m) => out.push(path.clone()),
                ModeTree::Leaf(_) => {}
                ModeTree::Record { elems, .. } => {
                    for (i, e) in elems.iter().enumerate() {
                        path.push(LspStep::Field(i as u32));
                        walk(e, path, out, keep);
                        path.pop();
                    }
                }
                ModeTree::Array(e) => walk(e, path, out, keep),
            }
        }
        fn below<'a>(mut t: &'a ModeTree, mut steps: &[LspStep]) -> &'a ModeTree {
            while let Some((s, rest)) = steps.split_first() {
                match (t, s) {
                    (ModeTree::Record { elems, .. }, LspStep::Field(i)) => t = &elems[*i as usize],
                    // The name addresses that field of every element.
                    (ModeTree::Array(e), LspStep::Field(_)) => {
                        t = e;
                        continue;
                    }
                    (ModeTree::Array(e), LspStep::Index(_) | LspStep::Slice(_)) => t = e,
                    (_, LspStep::Rebase(_)) => {}
                    // Below a leaf (an element of an array of scalars): the leaf's mode.
                    _ => return t,
                }
                steps = rest;
            }
            t
        }
        let mut out = Vec::new();
        walk(
            below(&self.port_mode_tree(m), steps),
            &mut Vec::new(),
            &mut out,
            keep,
        );
        out
    }

    /// The mode tree of view `v`, `converse` applied to every leaf.
    pub fn view_tree(&self, v: ViewId, converse: bool) -> ModeTree {
        let vd = &self.views[v];
        let elems = vd
            .elems
            .iter()
            .map(|e| match *e {
                ElemMode::Simple(m) => ModeTree::Leaf(if converse { converse_mode(m) } else { m }),
                ElemMode::View {
                    view,
                    converse: c,
                    array,
                } => {
                    let t = self.view_tree(view, converse != c);
                    if array {
                        ModeTree::Array(Box::new(t))
                    } else {
                        t
                    }
                }
            })
            .collect();
        ModeTree::Record {
            ty: self.base(vd.record),
            elems,
        }
    }

    /// The steps of name `e` below its root object, through object aliases, static or
    /// not: a target `bs(i).ready` drives only `ready`, whatever its longest static prefix.
    pub fn view_name_path(&self, e: ExprId) -> Vec<LspStep> {
        match &self.exprs[e].kind {
            ExprKind::Obj(d) => match &self.decls[*d].kind {
                DeclKind::Alias(crate::decl::Alias::Object { target, .. }) => {
                    self.view_name_path(*target)
                }
                _ => Vec::new(),
            },
            ExprKind::Field { prefix, elem } => {
                let mut p = self.view_name_path(*prefix);
                p.push(LspStep::Field(*elem));
                p
            }
            ExprKind::Index { prefix, indices } => {
                let mut p = self.view_name_path(*prefix);
                p.push(LspStep::Index(indices.clone()));
                p
            }
            ExprKind::Slice { prefix, range } => {
                let mut p = self.view_name_path(*prefix);
                p.push(LspStep::Slice(range.clone()));
                p
            }
            _ => Vec::new(),
        }
    }

    /// A port with a mode view is driven at `path` (a target's longest static
    /// prefix below the port): every element under it must be drivable, so no
    /// `in` element (`E1903`). Ports without a view pass.
    pub fn check_view_drive(
        &mut self,
        port: DeclId,
        path: &[LspStep],
        span: Span,
    ) -> Result<(), Reported> {
        let DeclKind::Object(o) = &self.decls[port].kind else {
            return Ok(());
        };
        let Some(m @ Mode::View { .. }) = o.mode else {
            return Ok(());
        };
        let tree = self.port_mode_tree(m);
        let sub = tree.select(path);
        let Some(names) = self.first_in_leaf(sub) else {
            return Ok(());
        };
        let elem = if names.is_empty() {
            String::new()
        } else {
            format!("element `{}` of ", names.join("."))
        };
        Err(self.error(
            Diagnostic::error(
                diag::VIEW_MODE,
                format!(
                    "{elem}port `{}` has mode `in` and cannot be driven",
                    self.decls[port].name
                ),
            )
            .with_primary(span, "driven here")
            .with_secondary(self.decls[port].span, "port declared here"),
        ))
    }

    /// The element names leading to the first `in` leaf of `t`.
    fn first_in_leaf(&self, t: &ModeTree) -> Option<Vec<String>> {
        match t {
            ModeTree::Leaf(m) => (*m == Mode::In).then(Vec::new),
            ModeTree::Array(e) => self.first_in_leaf(e),
            ModeTree::Record { ty, elems } => elems.iter().enumerate().find_map(|(i, e)| {
                let mut v = self.first_in_leaf(e)?;
                if let TypeKind::Record { elems } = &self.types[*ty].kind {
                    v.insert(0, elems[i].name.text.to_string());
                }
                Some(v)
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alias::fixture::*;
    use crate::decl::ObjClass;
    use crate::scope::ScopeKind;

    struct Cx {
        db: Db,
        s: ScopeId,
        rec: SubtypeId,
        inner: SubtypeId,
        decls: Vec<ast::Decl>,
    }

    fn cx(src: &str) -> Cx {
        let mut db = db_rev(LangRev::Vhdl2019);
        let s = region(&mut db, ScopeKind::Package);
        let (_, bit) = enum_type(&mut db, s, "bit", &["'0", "'1"]);
        let (_, inner) = record_type(&mut db, s, "inner", &[("x", bit), ("y", bit)]);
        let (_, rec) = record_type(&mut db, s, "rec", &[("a", bit), ("b", bit), ("i", inner)]);
        let decls = parse_decls(src, yavs_syntax::Std::Vhdl2019);
        Cx {
            db,
            s,
            rec,
            inner,
            decls,
        }
    }

    fn views(decls: &[ast::Decl]) -> Vec<&ast::ViewDecl> {
        decls
            .iter()
            .filter_map(|d| match d {
                ast::Decl::View(v) => Some(v),
                _ => None,
            })
            .collect()
    }

    /// Resolve `v` and `v'converse` by looking `v` up in region `s`.
    fn resolver(s: ScopeId) -> impl FnMut(&mut Db, &ast::Name) -> Result<ViewRef, Reported> {
        move |db: &mut Db, n: &ast::Name| {
            let (id, conv) = match &n.kind {
                ast::NameKind::Simple(i) => (*i, false),
                ast::NameKind::Attribute { prefix, .. } => match &prefix.kind {
                    ast::NameKind::Simple(i) => (*i, true),
                    _ => panic!(),
                },
                _ => panic!(),
            };
            let d = db.scopes[s].names[&id.key][0];
            db.view_ref(d, conv, n.span)
        }
    }

    fn declare_all(c: &mut Cx, subtypes: &[SubtypeId]) -> Vec<Result<DeclId, Reported>> {
        let vs: Vec<&ast::ViewDecl> = views(&c.decls);
        let mut out = Vec::new();
        for (v, &st) in vs.iter().zip(subtypes) {
            let mut r = resolver(c.s);
            let d = c.db.declare_view(c.s, v, st, &mut r);
            if let Ok(d) = d {
                let key = c.db.decls[d].name.key();
                c.db.scopes[c.s].names.entry(key).or_default().push(d);
            }
            out.push(d);
        }
        out
    }

    #[test]
    fn nested_views_converse_and_tree() {
        let src = "package p is
            view vi of inner is x : in; y : out; end view;
            view v of rec is a : in; b : buffer; i : view vi'converse; end view;
          end package;";
        let mut c = cx(src);
        let (inner, rec) = (c.inner, c.rec);
        let r = declare_all(&mut c, &[inner, rec]);
        let v = match c.db.decls[r[1].unwrap()].kind {
            DeclKind::View(v) => v,
            _ => panic!(),
        };
        let tree = c.db.view_tree(v, false);
        let leaves: Vec<Mode> = tree.leaves().into_iter().map(|(_, m)| m).collect();
        assert_eq!(leaves, [Mode::In, Mode::Buffer, Mode::Out, Mode::In]);
        // 'converse: in<->out, buffer stays (U-4), nested flips back.
        let cv = c.db.converse(v);
        assert_eq!(c.db.converse(v), cv);
        assert_eq!(c.db.converse(cv), v);
        let leaves: Vec<Mode> =
            c.db.view_tree(cv, false)
                .leaves()
                .into_iter()
                .map(|(_, m)| m)
                .collect();
        assert_eq!(leaves, [Mode::Out, Mode::Buffer, Mode::In, Mode::Out]);
        assert!(c.db.diagnostics().is_empty());
    }

    #[test]
    fn element_errors() {
        let src = "package p is
            view v1 of rec is a : in; a : out; b, i : out; end view;
            view v2 of rec is a : in; zz : out; b, i : out; end view;
            view v3 of rec is a : in; b : out; end view;
            view v4 of rec is a : in; b : linkage; i : out; end view;
            view vi of inner is x, y : in; end view;
            view v5 of rec is a : in; b : view vi; i : in; end view;
            view v6 of bit is end view;
          end package;";
        let mut c = cx(src);
        let (inner, rec) = (c.inner, c.rec);
        let bit =
            c.db.subtypes
                .iter()
                .find(|(_, s)| s.name.is_some_and(|n| n.text.as_str() == "bit"))
                .unwrap()
                .0;
        let r = declare_all(&mut c, &[rec, rec, rec, rec, inner, rec, bit]);
        assert_eq!(r.iter().filter(|x| x.is_err()).count(), 6);
        assert_eq!(
            codes(&c.db),
            [
                diag::VIEW_ELEMENT,
                diag::VIEW_ELEMENT,
                diag::VIEW_ELEMENT,
                diag::VIEW_ELEMENT,
                diag::VIEW_ELEMENT,
                diag::VIEW_NOT_RECORD
            ]
        );
    }

    #[test]
    fn ports_and_drives() {
        let src = "package p is
            view vi of inner is x : in; y : out; end view;
            view v of rec is a : in; b : out; i : view vi; end view;
          end package;";
        let mut c = cx(src);
        let (inner, rec) = (c.inner, c.rec);
        let r = declare_all(&mut c, &[inner, rec]);
        let vd = r[1].unwrap();
        let vr = c.db.view_ref(vd, false, sp(1)).unwrap();
        // A constant cannot have a view; a mismatched subtype is E1300.
        assert!(
            c.db.view_port(Some(ast::InterfaceClass::Constant), vr, false, None, sp(2))
                .is_err()
        );
        assert!(c.db.view_port(None, vr, false, Some(inner), sp(2)).is_err());
        let (mode, st) = c.db.view_port(None, vr, false, None, sp(2)).unwrap();
        assert_eq!(st, rec);
        let (port, _) = object(&mut c.db, c.s, "x", ObjClass::Signal, rec);
        if let DeclKind::Object(o) = &mut c.db.decls[port].kind {
            o.mode = Some(mode);
        }
        assert!(
            c.db.check_view_drive(port, &[LspStep::Field(1)], sp(3))
                .is_ok()
        );
        assert!(
            c.db.check_view_drive(port, &[LspStep::Field(2), LspStep::Field(1)], sp(3))
                .is_ok()
        );
        assert!(
            c.db.check_view_drive(port, &[LspStep::Field(0)], sp(3))
                .is_err()
        );
        assert!(c.db.check_view_drive(port, &[], sp(3)).is_err());
        let msg = &c.db.diagnostics().last().unwrap().message;
        assert_eq!(
            msg,
            "element `a` of port `x` has mode `in` and cannot be driven"
        );
        // Not a view.
        let bitlit = c.db.scopes[c.s].names[&yavs_syntax::Symbol::intern("'0'")][0];
        assert!(c.db.view_ref(bitlit, false, sp(4)).is_err());
        assert_eq!(
            codes(&c.db),
            [
                diag::VIEW_MODE,
                diag::TYPE_MISMATCH,
                diag::VIEW_MODE,
                diag::VIEW_MODE,
                diag::VIEW_NOT_RECORD
            ]
        );
    }

    #[test]
    fn array_view_elements() {
        let mut db = db_rev(LangRev::Vhdl2019);
        let s = region(&mut db, ScopeKind::Package);
        let (_, bit) = enum_type(&mut db, s, "bit", &["'0", "'1"]);
        let (_, int) = int_type(&mut db, s, "integer");
        let (_, inner) = record_type(&mut db, s, "inner", &[("x", bit)]);
        let (_, arr) = array_type(&mut db, s, "inner_vec", int, inner);
        let (_, rec) = record_type(&mut db, s, "rec", &[("v", arr)]);
        let decls = parse_decls(
            "package p is view vi of inner is x : out; end view; view v of rec is v : view (vi); end view; end package;",
            yavs_syntax::Std::Vhdl2019,
        );
        let vs = views(&decls);
        let mut r = resolver(s);
        let vi = db.declare_view(s, vs[0], inner, &mut r).unwrap();
        let k = db.decls[vi].name.key();
        db.scopes[s].names.entry(k).or_default().push(vi);
        let v = db.declare_view(s, vs[1], rec, &mut r).unwrap();
        let DeclKind::View(v) = db.decls[v].kind else {
            panic!()
        };
        let leaves = db.view_tree(v, false).leaves();
        assert_eq!(
            leaves,
            [(
                vec![ViewStep::Field(0), ViewStep::Elements, ViewStep::Field(0)],
                Mode::Out
            )]
        );
        // `p : view (vi) of inner_vec`.
        let vr = db.view_ref(vi, false, sp(1)).unwrap();
        let (m, st) = db.view_port(None, vr, true, Some(arr), sp(1)).unwrap();
        assert_eq!(st, arr);
        assert!(matches!(db.port_mode_tree(m), ModeTree::Array(_)));
        assert!(db.view_port(None, vr, true, Some(rec), sp(1)).is_err());
        assert_eq!(codes(&db), [diag::TYPE_MISMATCH]);
    }
}
