//! Design units, context clauses and configurations (sem.md 1.2, 1.4, 6.5).

use yavs_diag::{Diagnostic, FileId, Span};
use yavs_syntax::Symbol;
use yavs_syntax::ast::{self, Ident};

use crate::LangRev;
use crate::db::{Db, Reported, UnitCx};
use crate::decl::{Decl, DeclKind, Designator};
use crate::diag;
use crate::expr::names::NameRes;
use crate::hir::UnitHir;
use crate::ids::{DeclId, LibId, ScopeId, UnitId};
use crate::loader::{Unit, UnitKey, UnitKind, UnitSlot};
use crate::scope::{ScopeKind, UseItem, Visible};

mod binding;
pub(crate) use binding::PendingSpec;

#[cfg(test)]
mod tests;

/// What the prefix of an expanded name or use clause denotes.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Region {
    Library(LibId),
    /// A package, package instance or other region whose declarations can be selected.
    Scope(ScopeId),
}

fn ident(text: &str, span: Span) -> Ident {
    Ident {
        text: Symbol::intern(text),
        key: Symbol::intern(&text.to_ascii_lowercase()),
        span,
    }
}

impl Db {
    /// Analyze one design unit whose AST was taken from the file cache.
    pub(crate) fn analyze_unit(
        &mut self,
        lib: LibId,
        key: UnitKey,
        ast: ast::DesignUnit,
        file: FileId,
        rev: LangRev,
    ) -> Result<UnitId, Reported> {
        use ast::LibraryUnit as L;
        let (kind, name) = match &ast.unit {
            L::Entity(e) => (UnitKind::Entity, e.name),
            L::Architecture(a) => (UnitKind::Architecture, a.name),
            L::Package(p) => (UnitKind::Package, p.name),
            L::PackageBody(b) => (UnitKind::PackageBody, b.name),
            L::PackageInstance(p) => (UnitKind::PackageInst, p.name),
            L::Configuration(c) => (UnitKind::Configuration, c.name),
            L::Context(c) => (UnitKind::Context, c.name),
        };
        let root = self.root;
        let unit = self.units.push(Unit {
            lib,
            key,
            kind,
            name,
            decl: self.poison.decl,
            scope: root,
            context: root,
            deps: Vec::new(),
            refs: Vec::new(),
            file,
            span: ast.span,
            rev,
            hir: UnitHir::Pending,
        });
        self.libs[lib].units.insert(key, UnitSlot::InProgress(unit));
        self.stack.push(UnitCx {
            unit,
            lib,
            file_rev: rev,
        });
        let before = self.error_count();
        // The memos are keyed by AST node address: entries must not outlive this unit's
        // AST, or a later allocation at the same address reads stale candidates.
        let memos = (
            std::mem::take(&mut self.memo),
            std::mem::take(&mut self.name_memo),
            std::mem::take(&mut self.ext_memo),
        );
        let outer_reads = std::mem::take(&mut self.out_reads);
        let r = self.unit_body(unit, &ast);
        // After another error the references an aborted statement left are no evidence (8.2).
        if r.is_ok() && self.error_count() == before {
            self.report_out_reads();
        }
        self.out_reads = outer_reads;
        (self.memo, self.name_memo, self.ext_memo) = memos;
        self.stack.pop();
        let ok = r.is_ok() && self.error_count() == before;
        self.libs[lib].units.insert(
            key,
            if ok {
                UnitSlot::Done(unit)
            } else {
                UnitSlot::Failed
            },
        );
        if ok { Ok(unit) } else { Err(Reported) }
    }

    fn unit_body(&mut self, unit: UnitId, ast: &ast::DesignUnit) -> Result<(), Reported> {
        use ast::LibraryUnit as L;
        let lib = self.units[unit].lib;
        let name = self.units[unit].name;
        // Checked before the primary lookup; a name that may come from a context reference
        // is checked again once the context scope exists (below).
        let ctx_ref = ast
            .context
            .iter()
            .any(|c| matches!(c, ast::ContextItem::Context { .. }));
        if let L::Architecture(a) = &ast.unit
            && let Some(l) = a.entity_lib
        {
            self.arch_entity_lib(lib, l, a.entity, &ast.context, None)?;
        }
        // Secondary units start from their primary's region.
        let primary = match &ast.unit {
            L::Architecture(a) => Some((a.entity, UnitKind::Entity)),
            L::PackageBody(b) => Some((b.name, UnitKind::Package)),
            _ => None,
        };
        let mut parent = None;
        if let Some((pname, want)) = primary {
            let pk = UnitKey::Primary(pname.key);
            if !self.unit_known(lib, pk) {
                let what = if want == UnitKind::Entity {
                    "entity"
                } else {
                    "package"
                };
                return Err(self.error(
                    Diagnostic::error(
                        diag::NO_PRIMARY,
                        format!(
                            "{what} `{}` not found in library `{}`",
                            pname.text, self.libs[lib].name.text
                        ),
                    )
                    .with_primary(pname.span, format!("no {what} with this name"))
                    .with_help(format!("analyze the {what} before its secondary units")),
                ));
            }
            let p = self.load_unit(lib, pk, Some(pname.span))?;
            if self.units[p].kind != want {
                let have = kind_str(self.units[p].kind);
                return Err(self.error(
                    Diagnostic::error(
                        diag::NO_PRIMARY,
                        format!("`{}` is a {have}, not {}", pname.text, article(want)),
                    )
                    .with_primary(pname.span, format!("expected {}", article(want)))
                    .with_secondary(
                        self.units[p].name.span,
                        format!("the {have} is declared here"),
                    ),
                ));
            }
            self.units[unit].deps.push(p);
            // The primary's context is this unit's too: a failed use clause there taints both (8.2).
            if self.tainted.contains(&p) {
                self.tainted.insert(unit);
            }
            parent = Some(self.units[p].scope);
        }
        let context = self.add_scope(ScopeKind::Context, parent, None);
        let decl = self.decls.push(Decl {
            name: Designator::Ident(name),
            kind: DeclKind::Unit(unit),
            scope: context,
            span: name.span,
            implicit: false,
            since: LangRev::Vhdl1993,
        });
        self.scopes[context].owner = Some(decl);
        self.units[unit].context = context;
        self.units[unit].decl = decl;
        self.implicit_context(unit, context, ast.span)?;
        for item in &ast.context {
            self.context_item(context, item, false);
        }
        if let L::Architecture(a) = &ast.unit
            && let Some(l) = a.entity_lib
            && ctx_ref
        {
            self.arch_entity_lib(lib, l, a.entity, &ast.context, Some(context))?;
        }
        // The architecture's name belongs to the library, outside the entity's region, so a
        // port or generic of the same name hides it inside the architecture (12.3).
        // ponytail: skipped instead of declared further out; `arch.x` expanded names then
        // fail only when a port has the architecture's name.
        // The same holds for a package body and an item of its package with the package's name
        // (#471).
        let hidden = matches!(ast.unit, L::Architecture(_) | L::PackageBody(_))
            && parent.is_some_and(|e| self.scopes[e].names.contains_key(&name.key));
        if !hidden {
            self.declare(context, decl);
        }
        let scope_kind = match &ast.unit {
            L::Entity(_) => ScopeKind::Entity,
            L::Architecture(_) => ScopeKind::Architecture,
            L::Package(_) | L::PackageInstance(_) => ScopeKind::Package,
            L::PackageBody(_) => ScopeKind::PackageBody,
            L::Configuration(_) => ScopeKind::Configuration,
            L::Context(_) => ScopeKind::Context,
        };
        let scope = self.add_scope(scope_kind, Some(context), Some(decl));
        self.units[unit].scope = scope;
        let hir = match &ast.unit {
            L::Package(p) if p.generic_map.is_some() => {
                UnitHir::PackageInst(self.mapped_package(context, scope, decl, p)?)
            }
            L::Package(p) => {
                let generics = p.generics.as_ref().map(|g| self.generic_list(scope, g));
                let is_standard = self.is_std_standard(unit);
                if is_standard {
                    self.begin_standard(scope);
                }
                self.decls_in(scope, &p.decls);
                if is_standard {
                    self.end_standard(scope, p.span);
                }
                let generics = generics.transpose()?;
                let lib = self.units[unit].lib;
                let body = UnitKey::body(name.key);
                if let Some(g) = &generics
                    && !self.unit_known(lib, body)
                {
                    self.report_unused_generics(g);
                }
                UnitHir::Package { generics }
            }
            L::PackageBody(b) => {
                let package = self.units[unit].deps[0];
                if matches!(self.units[package].hir, UnitHir::PackageInst(_)) {
                    return Err(self.unsupported(
                        b.name.span,
                        "a package body of a package with a generic map",
                        "S4",
                    ));
                }
                self.decls_in(scope, &b.decls);
                let pscope = self.units[package].scope;
                let _ = self.check_protected_bodies(pscope, b.span);
                if let UnitHir::Package { generics: Some(g) } = &self.units[package].hir {
                    let g = g.clone();
                    self.report_unused_generics(&g);
                }
                UnitHir::PackageBody { package }
            }
            L::Context(c) => {
                for item in &c.items {
                    self.context_item(scope, item, true);
                }
                UnitHir::Context
            }
            L::Entity(e) => self.entity_body(unit, scope, e),
            L::Architecture(a) => self.architecture_body(unit, scope, a),
            L::Configuration(c) => self.configuration_body(unit, scope, c)?,
            L::PackageInstance(p) => {
                UnitHir::PackageInst(self.package_instance_unit(scope, decl, p)?)
            }
        };
        self.units[unit].hir = hir;
        Ok(())
    }

    fn is_std_standard(&self, unit: UnitId) -> bool {
        let u = &self.units[unit];
        self.libs[u.lib].name.key.as_str() == "std"
            && u.key == UnitKey::Primary(Symbol::intern("standard"))
    }

    fn wrong_kind(&mut self, u: UnitId, span: Span, want: UnitKind) -> Reported {
        let have = self.units[u].kind;
        self.error(
            Diagnostic::error(
                diag::WRONG_UNIT_KIND,
                format!(
                    "`{}` is {}, not {}",
                    self.units[u].name.text,
                    article(have),
                    article(want)
                ),
            )
            .with_primary(span, format!("expected {}", article(want)))
            .with_secondary(
                self.units[u].name.span,
                format!("the {} is declared here", kind_str(have)),
            ),
        )
    }

    /// `library std, work; use std.standard.all;` (13.2).
    fn implicit_context(
        &mut self,
        unit: UnitId,
        context: ScopeId,
        span: Span,
    ) -> Result<(), Reported> {
        let std_lib = self.library(ident("std", Span::default()))?;
        let lib = self.units[unit].lib;
        // A secondary unit sits inside its primary's region, which sees the same implicit
        // names through its own context: declared again here they would hide what the
        // primary declares under the name `work` or `std` (12.3, #640).
        let secondary = self.scopes[context].parent.is_some();
        if !secondary {
            let std_decl = self.libs[std_lib].decl;
            self.scopes[context]
                .names
                .entry(Symbol::intern("std"))
                .or_default()
                .push(std_decl);
            let work = self.decls.push(Decl {
                name: Designator::Ident(ident("work", Span::default())),
                kind: DeclKind::Library(lib),
                scope: context,
                span: Span::default(),
                implicit: true,
                since: LangRev::Vhdl1993,
            });
            self.scopes[context]
                .names
                .entry(Symbol::intern("work"))
                .or_default()
                .push(work);
        }
        if !self.is_std_standard(unit) {
            let standard = self.load_unit(
                std_lib,
                UnitKey::Primary(Symbol::intern("standard")),
                Some(span),
            )?;
            self.units[unit].deps.push(standard);
            if !secondary {
                let s = self.units[standard].scope;
                self.scopes[context].uses.push(UseItem::All(s));
            }
        }
        Ok(())
    }

    /// One library, use or context item, into `region`.
    pub(crate) fn context_item(
        &mut self,
        region: ScopeId,
        item: &ast::ContextItem,
        in_context_decl: bool,
    ) {
        match item {
            ast::ContextItem::Library { names, .. } => {
                for &n in names {
                    if in_context_decl && n.key.as_str() == "work" {
                        self.report(
                            Diagnostic::error(diag::WORK_IN_CONTEXT, "a context declaration cannot name library `work`")
                                .with_primary(n.span, "not allowed in a context declaration")
                                .with_note("a context declaration is used from units in other libraries, so `work` would mean different libraries (LRM 13.4)"),
                        );
                        continue;
                    }
                    let lib = if n.key.as_str() == "work" {
                        match self.cur() {
                            Some(c) => c.lib,
                            None => continue,
                        }
                    } else {
                        match self.library(n) {
                            Ok(l) => l,
                            Err(_) => continue,
                        }
                    };
                    let already = self.scopes[region].names.get(&n.key).is_some_and(|ds| {
                        ds.iter().any(
                            |&d| matches!(self.decls[d].kind, DeclKind::Library(l) if l == lib),
                        )
                    });
                    if !already {
                        let d = self.decls.push(Decl {
                            name: Designator::Ident(n),
                            kind: DeclKind::Library(lib),
                            scope: region,
                            span: n.span,
                            implicit: false,
                            since: LangRev::Vhdl1993,
                        });
                        self.declare(region, d);
                    }
                }
            }
            ast::ContextItem::Use(u) => {
                if in_context_decl {
                    for n in &u.names {
                        self.check_no_work_prefix(n);
                    }
                }
                self.use_clause(region, u);
            }
            ast::ContextItem::Context { names, .. } => {
                for n in names {
                    if in_context_decl {
                        self.check_no_work_prefix(n);
                    }
                    let _ = self.context_reference(region, n);
                }
            }
        }
    }

    fn check_no_work_prefix(&mut self, n: &ast::Name) {
        let mut p = n;
        while let ast::NameKind::Selected { prefix, .. } = &p.kind {
            p = prefix;
        }
        if let ast::NameKind::Simple(i) = &p.kind
            && i.key.as_str() == "work"
        {
            self.report(
                Diagnostic::error(
                    diag::WORK_IN_CONTEXT,
                    "a context declaration cannot refer to library `work`",
                )
                .with_primary(i.span, "not allowed in a context declaration"),
            );
        }
    }

    /// `context l.c;`: splice the items of context declaration `c` (13.4).
    fn context_reference(&mut self, region: ScopeId, n: &ast::Name) -> Result<(), Reported> {
        let d = self.expanded_name_decl(region, n)?;
        let u = match self.decls[d].kind {
            DeclKind::Unit(u) if self.units[u].kind == UnitKind::Context => u,
            DeclKind::Error => return Err(Reported),
            _ => {
                return Err(self.error(
                    Diagnostic::error(
                        diag::NOT_A_PACKAGE,
                        format!("`{}` is not a context declaration", self.decls[d].name),
                    )
                    .with_primary(crate::generic::last_span(n), "not a context declaration")
                    .with_secondary(self.decls[d].span, "declared here"),
                ));
            }
        };
        let s = self.units[u].scope;
        let names: Vec<(Symbol, Vec<DeclId>)> = self.scopes[s]
            .names
            .iter()
            .map(|(k, v)| (*k, v.clone()))
            .collect();
        for (k, ds) in names {
            for d in ds {
                let dup = self.scopes[region]
                    .names
                    .get(&k)
                    .is_some_and(|v| v.contains(&d));
                if !dup {
                    self.scopes[region].names.entry(k).or_default().push(d);
                }
            }
        }
        let uses = self.scopes[s].uses.clone();
        self.scopes[region].uses.extend(uses);
        Ok(())
    }

    /// A use clause in a context clause or declarative part (12.4).
    pub(crate) fn use_clause(&mut self, region: ScopeId, u: &ast::UseClause) {
        for n in &u.names {
            let ast::NameKind::Selected { prefix, suffix } = &n.kind else {
                self.report(
                    Diagnostic::error(diag::NOT_A_PACKAGE, "a use clause needs a selected name")
                        .with_primary(n.span, "expected `library.package.all` or `package.name`"),
                );
                continue;
            };
            let Ok(r) = self.region_of(region, prefix) else {
                continue;
            };
            let item = match (r, suffix) {
                (Region::Library(l), ast::Suffix::All(_)) => UseItem::Library(l),
                (Region::Scope(s), ast::Suffix::All(_)) => UseItem::All(s),
                (Region::Library(l), suffix) => {
                    let Some(key) = suffix_key(suffix) else {
                        continue;
                    };
                    let at = suffix_span(suffix, n.span);
                    let Ok(u) = self.load_dep(l, UnitKey::Primary(key), Some(at)) else {
                        continue;
                    };
                    self.note_dep(u);
                    UseItem::One(vec![self.units[u].decl])
                }
                (Region::Scope(s), suffix) => {
                    let Some(key) = suffix_key(suffix) else {
                        continue;
                    };
                    match self.scopes[s].names.get(&key) {
                        Some(ds) if !ds.is_empty() => {
                            let ds = ds.clone();
                            let at = suffix_span(suffix, n.span);
                            let ds = match self.rev_gate(NameRes::Overloads(ds), n.span, at) {
                                NameRes::Overloads(ds) => ds,
                                _ => continue,
                            };
                            UseItem::One(self.with_type_associates(s, ds))
                        }
                        _ => {
                            self.not_in_region(s, suffix_span(suffix, n.span), key);
                            continue;
                        }
                    }
                }
            };
            self.scopes[region].uses.push(item);
        }
    }

    /// 12.4: a use clause naming a type also makes its enumeration literals, its
    /// units and its operations potentially visible, the implicitly declared ones and the
    /// operators of the same region that take or return the type (#452). Other subprograms
    /// of the region stay hidden: all four reference simulators reject their use (#514).
    fn with_type_associates(&self, region: ScopeId, mut ds: Vec<DeclId>) -> Vec<DeclId> {
        let types: Vec<crate::ids::TypeId> = ds
            .iter()
            .filter_map(|&d| match self.decls[d].kind {
                DeclKind::Type(t) => Some(t),
                DeclKind::Subtype(st) => Some(self.base(st)),
                _ => None,
            })
            .collect();
        if types.is_empty() {
            return ds;
        }
        for &d in &self.scopes[region].order {
            let assoc = match &self.decls[d].kind {
                DeclKind::EnumLit { ty, .. } | DeclKind::PhysUnit { ty, .. } => types.contains(ty),
                DeclKind::Subprogram(sp)
                    if self.decls[d].implicit
                        || matches!(self.decls[d].name, Designator::Operator { .. }) =>
                {
                    sp.params
                        .iter()
                        .any(|p| types.contains(&self.base(p.subtype)))
                        || sp.ret.is_some_and(|r| types.contains(&self.base(r)))
                }
                _ => false,
            };
            if assoc && !ds.contains(&d) {
                ds.push(d);
            }
        }
        ds
    }

    pub(crate) fn note_dep(&mut self, u: UnitId) {
        if let Some(c) = self.cur()
            && c.unit != u
            && !self.units[c.unit].deps.contains(&u)
        {
            self.units[c.unit].deps.push(u);
        }
    }

    /// `key` is not usable from region `s` in an expanded name or use clause: not
    /// declared there (`E1100`), declared in the package's body (`E1105`, 12.3), or
    /// declared in a construct that does not enclose the name (`E1103`, 8.3).
    pub(crate) fn not_in_region(&mut self, s: ScopeId, span: Span, key: Symbol) -> Reported {
        let owner = self.scopes[s]
            .owner
            .map(|o| self.decls[o].name.to_string())
            .unwrap_or_default();
        if let Some(&d) = self.scopes[s].names.get(&key).and_then(|v| v.first()) {
            let name = self.decls[d].name.to_string();
            let what = match self.scopes[s].kind {
                ScopeKind::Process => "process",
                ScopeKind::Subprogram => "subprogram",
                ScopeKind::Block => "block",
                ScopeKind::Generate => "generate statement",
                ScopeKind::Loop => "loop",
                ScopeKind::Protected | ScopeKind::ProtectedBody => "protected type",
                _ => "construct",
            };
            return self.error(
                Diagnostic::error(
                    diag::HIDDEN,
                    format!("`{name}` exists but is not visible here"),
                )
                .with_primary(span, "not visible here")
                .with_secondary(self.decls[d].span, "declared here")
                .with_help(format!(
                    "`{name}` is declared in {what} `{owner}` and is only visible inside it"
                )),
            );
        }
        if self.scopes[s].kind == ScopeKind::Package
            && let Some(o) = self.scopes[s].owner
            && let DeclKind::Unit(u) = self.decls[o].kind
            && let Some(UnitSlot::Done(b)) = self.libs[self.units[u].lib]
                .units
                .get(&UnitKey::body(self.units[u].name.key))
            && let Some(&d) = self.scopes[self.units[*b].scope]
                .names
                .get(&key)
                .and_then(|v| v.first())
        {
            return self.error(
                Diagnostic::error(
                    diag::PRIVATE,
                    format!("`{key}` is not in the visible part of `{owner}`"),
                )
                .with_primary(span, "declared in the package body")
                .with_secondary(self.decls[d].span, "declared here"),
            );
        }
        let d = Diagnostic::error(
            diag::NOT_DECLARED,
            format!("`{key}` is not declared in `{owner}`"),
        )
        .with_primary(span, "not found");
        let found = crate::suggest::similar(
            key.as_str(),
            self.scopes[s]
                .names
                .values()
                .filter_map(|v| self.candidate(*v.first()?)),
        );
        let d = self.suggest_decls(d, span, &found);
        self.error(d)
    }

    /// The declarative region of the entity or architecture `d` names, when `scope` lies
    /// inside it: its own declarations can be selected with its name (8.3, #639).
    pub(crate) fn enclosing_unit_region(&self, scope: ScopeId, d: DeclId) -> Option<ScopeId> {
        let DeclKind::Unit(u) = self.decls[self.entity(d)].kind else {
            return None;
        };
        if !matches!(
            self.units[u].kind,
            UnitKind::Entity | UnitKind::Architecture
        ) {
            return None;
        }
        let region = self.units[u].scope;
        let mut cur = Some(scope);
        while let Some(c) = cur {
            if c == region {
                return Some(region);
            }
            cur = self.scopes[c].parent;
        }
        None
    }

    /// The region a name denotes, for use clauses and expanded names: a library,
    /// a package (loaded on demand) or a package instance.
    pub(crate) fn region_of(&mut self, scope: ScopeId, n: &ast::Name) -> Result<Region, Reported> {
        let d = self.expanded_name_decl(scope, n)?;
        if let Some(s) = self.enclosing_unit_region(scope, d) {
            return Ok(Region::Scope(s));
        }
        // A label of an enclosing statement names its declarative region (8.3): `p.st` inside
        // process `p`.
        if let DeclKind::Label(_) = self.decls[d].kind
            && let Some((s, true)) = self.construct_region(scope, &[d])
        {
            return Ok(Region::Scope(s));
        }
        match self.decls[d].kind {
            DeclKind::Library(l) => Ok(Region::Library(l)),
            DeclKind::Unit(_)
            | DeclKind::PackageInst(_)
            | DeclKind::GenericPackage(_)
            | DeclKind::Package { .. }
                if !matches!(self.package_region(d), crate::generic::PkgRegion::No) =>
            {
                match self.package_region(d) {
                    crate::generic::PkgRegion::Scope(s) => Ok(Region::Scope(s)),
                    _ => Err(self.uninstantiated(d, crate::generic::last_span(n))),
                }
            }
            DeclKind::Error => Err(Reported),
            _ => Err(self.error(
                Diagnostic::error(
                    diag::NOT_A_PACKAGE,
                    format!(
                        "`{}` is not a package, library or package instance",
                        self.decls[d].name
                    ),
                )
                .with_primary(
                    crate::generic::last_span(n),
                    "expected a library or a package",
                )
                .with_secondary(self.decls[d].span, "declared here"),
            )),
        }
    }

    /// Resolve a simple or selected name that must denote exactly one
    /// non-overloadable declaration (library, unit, package, type, ...).
    pub(crate) fn expanded_name_decl(
        &mut self,
        scope: ScopeId,
        n: &ast::Name,
    ) -> Result<DeclId, Reported> {
        match &n.kind {
            ast::NameKind::Simple(i) => match self.lookup(scope, i.key) {
                Visible::One(d) => Ok(d),
                Visible::None => Err(self.undeclared(scope, *i)),
                Visible::Hidden(c) => Err(self.conflict(*i, &c)),
                Visible::Overloads(ds) => Ok(ds[0]),
            },
            ast::NameKind::Selected { prefix, suffix } => {
                let r = self.region_of(scope, prefix)?;
                let key = suffix_key(suffix).ok_or_else(|| {
                    self.error(
                        Diagnostic::error(
                            diag::NOT_A_PACKAGE,
                            "`all` is only allowed in a use clause",
                        )
                        .with_primary(n.span, "unexpected `.all`"),
                    )
                })?;
                match r {
                    Region::Library(l) => {
                        let at = suffix_span(suffix, n.span);
                        let u = self.load_dep(l, UnitKey::Primary(key), Some(at))?;
                        self.note_dep(u);
                        Ok(self.units[u].decl)
                    }
                    Region::Scope(s) => match self.scopes[s].names.get(&key) {
                        Some(ds) if !ds.is_empty() => Ok(ds[0]),
                        _ => Err(self.not_in_region(s, suffix_span(suffix, n.span), key)),
                    },
                }
            }
            _ => Err(self.error(
                Diagnostic::error(diag::NOT_A_PACKAGE, "expected a simple or selected name")
                    .with_primary(n.span, "not a name of a library, package or unit"),
            )),
        }
    }
}

pub(crate) fn suffix_key(s: &ast::Suffix) -> Option<Symbol> {
    match s {
        ast::Suffix::Ident(i) => Some(i.key),
        ast::Suffix::Char(c) => Some(crate::decl::char_key(c.value)),
        ast::Suffix::Operator(op, _) => Some(crate::decl::operator_key(op.as_str())),
        ast::Suffix::All(_) => None,
    }
}

pub(crate) fn suffix_span(s: &ast::Suffix, whole: Span) -> Span {
    match s {
        ast::Suffix::Ident(i) => i.span,
        ast::Suffix::Char(c) => c.span,
        ast::Suffix::Operator(_, sp) | ast::Suffix::All(sp) => *sp,
    }
    .max_or(whole)
}

trait MaxOr {
    fn max_or(self, other: Span) -> Span;
}

impl MaxOr for Span {
    fn max_or(self, other: Span) -> Span {
        if self == Span::default() { other } else { self }
    }
}

/// `an entity`, `a package`, ...
fn article(k: UnitKind) -> String {
    let s = kind_str(k);
    let a = if s.starts_with(['a', 'e', 'i', 'o', 'u']) {
        "an"
    } else {
        "a"
    };
    format!("{a} {s}")
}

fn kind_str(k: UnitKind) -> &'static str {
    match k {
        UnitKind::Entity => "entity",
        UnitKind::Architecture => "architecture",
        UnitKind::Package => "package",
        UnitKind::PackageBody => "package body",
        UnitKind::PackageInst => "package instance",
        UnitKind::Context => "context declaration",
        UnitKind::Configuration => "configuration",
    }
}

impl Db {
    /// Start of `std.standard`: the universal types (5.2.3.1, 5.2.5.1). They have
    /// no name a design can write; their implicit operations are declared in
    /// `std.standard` like any type's.
    pub(crate) fn begin_standard(&mut self, scope: ScopeId) {
        use crate::types::{Constraint, RangeC, TypeKind};
        self.well_known.standard = Some(scope);
        for (name, kind) in [
            ("universal_integer", TypeKind::UniversalInteger),
            ("universal_real", TypeKind::UniversalReal),
        ] {
            let id = ident(name, Span::default());
            let is_int = matches!(kind, TypeKind::UniversalInteger);
            let decl = self.decls.push(Decl {
                name: Designator::Ident(id),
                kind: DeclKind::Error,
                scope,
                span: Span::default(),
                implicit: true,
                since: LangRev::Vhdl1993,
            });
            let ty = self.new_type(decl, id, false, kind, Constraint::None, Span::default());
            self.decls[decl].kind = DeclKind::Type(ty);
            let st = self.types[ty].first_subtype;
            let (lo, hi) = if is_int {
                (
                    crate::stat::ConstValue::Int(i64::MIN),
                    crate::stat::ConstValue::Int(i64::MAX),
                )
            } else {
                (
                    crate::stat::ConstValue::Real(f64::MIN),
                    crate::stat::ConstValue::Real(f64::MAX),
                )
            };
            let left = self.const_expr(lo, st, Span::default());
            let right = self.const_expr(hi, st, Span::default());
            self.subtypes[st].constraint = Constraint::Range(RangeC::Explicit {
                left,
                dir: crate::types::Direction::To,
                right,
            });
            if is_int {
                self.well_known.universal_integer = Some(ty);
            } else {
                self.well_known.universal_real = Some(ty);
            }
            self.scopes[scope].order.push(decl);
            self.declare_implicit_ops(ty, scope);
        }
    }

    /// End of `std.standard`: every queued implicit operation must be created by now.
    pub(crate) fn end_standard(&mut self, _scope: ScopeId, span: Span) {
        if !self.pending_implicit.is_empty() {
            self.ice(
                span,
                "implicit operations of std.standard still pending at its end",
            );
        }
    }
}
