//! Entities, architectures, components, instances, generic and port maps,
//! configuration specifications and declarations, default binding
//! (sem.md 6.5, 7.2; LRM 3, 6.5.6, 6.5.7, 7.3, 11.7).

use rsim_diag::{Diagnostic, Span};
use rsim_lib::{UnitKind as LibKind, UnitRef};
use rsim_syntax::Symbol;
use rsim_syntax::ast::{self, Ident};

use crate::db::{Db, Reported};
use crate::decl::{DeclKind, ObjClass, ObjRole};
use crate::diag;
use crate::hir::{
    Actual, Arg, Binding, BlockConfig, BlockTarget, ComponentConfig, ConfigItem, EntityAspect,
    ExprKind, GenSelect, InstTarget, InstanceSel, Mode, ObjRoot, StmtKind, UnitHir,
};
use crate::ids::{DeclId, LibId, ScopeId, SubtypeId, UnitId};
use crate::loader::{LibSource, UnitKey, UnitKind, UnitSlot};
use crate::scope::{ScopeKind, UseItem};

/// A configuration specification whose instance labels are resolved once
/// the statement part of its region has been analyzed.
pub(crate) struct PendingSpec {
    unit: UnitId,
    /// The region the specification is declared in (its labels are declared there).
    scope: ScopeId,
    labels: Labels,
    component: DeclId,
    binding: Binding,
    span: Span,
}

enum Labels {
    Some(Vec<Ident>),
    All,
    Others,
}

impl Labels {
    fn of(l: &ast::InstanceList) -> Labels {
        match l {
            ast::InstanceList::Labels(v) => Labels::Some(v.clone()),
            ast::InstanceList::All => Labels::All,
            ast::InstanceList::Others => Labels::Others,
        }
    }
}

/// Collects errors so independent ones are all reported; the result is still `Err`.
#[derive(Default)]
struct Errs(bool);

impl Errs {
    fn keep<T>(&mut self, r: Result<T, Reported>) -> Option<T> {
        r.map_err(|_| self.0 = true).ok()
    }
    fn finish<T>(self, v: T) -> Result<T, Reported> {
        if self.0 { Err(Reported) } else { Ok(v) }
    }
}

/// How one formal is associated.
enum Assoc {
    None,
    /// The whole formal, from association `idx`, with a formal conversion.
    Whole {
        idx: usize,
        conv: Option<DeclId>,
        span: Span,
    },
    /// Sub-element associations: the formal names and their association indexes.
    Parts(Vec<(usize, Span)>),
}

impl Db {
    // -- design units -------------------------------------------------------------

    /// An entity declaration (3.2): generics, ports, declarations and the
    /// passive statement part.
    pub(super) fn entity_body(
        &mut self,
        unit: UnitId,
        scope: ScopeId,
        e: &ast::EntityDecl,
    ) -> UnitHir {
        let generics = match &e.generics {
            Some(l) => self
                .interface_list(scope, l, ObjRole::Generic)
                .unwrap_or_default(),
            None => Box::default(),
        };
        let ports = match &e.ports {
            Some(l) => self
                .interface_list(scope, l, ObjRole::Port)
                .unwrap_or_default(),
            None => Box::default(),
        };
        self.decls_in(scope, &e.decls);
        let _ = self.check_protected_bodies(scope, e.span);
        // Set before the statements: their analysis never needs it, but a
        // failed statement part must still leave the interface visible.
        self.units[unit].hir = UnitHir::Entity {
            generics: generics.clone(),
            ports: ports.clone(),
            stmts: Box::default(),
        };
        let stmts = self
            .analyze_conc_stmts(scope, &e.stmts, true)
            .unwrap_or_default();
        UnitHir::Entity {
            generics,
            ports,
            stmts,
        }
    }

    /// An architecture body (3.3); its region continues the entity's.
    pub(super) fn architecture_body(
        &mut self,
        unit: UnitId,
        scope: ScopeId,
        a: &ast::ArchitectureBody,
    ) -> UnitHir {
        let entity = self.units[unit].deps[0];
        self.decls_in(scope, &a.decls);
        let _ = self.check_protected_bodies(scope, a.span);
        let stmts = self
            .analyze_conc_stmts(scope, &a.stmts, false)
            .unwrap_or_default();
        self.resolve_pending_specs(unit);
        UnitHir::Architecture { entity, stmts }
    }

    /// A configuration declaration (3.4).
    pub(super) fn configuration_body(
        &mut self,
        unit: UnitId,
        scope: ScopeId,
        c: &ast::ConfigurationDecl,
    ) -> Result<UnitHir, Reported> {
        let lib = self.units[unit].lib;
        let entity = match &c.entity.kind {
            ast::NameKind::Simple(i) => {
                let u = self.load_dep(lib, UnitKey::Primary(i.key), Some(i.span))?;
                if self.units[u].kind != UnitKind::Entity {
                    return Err(self.wrong_kind(u, i.span, UnitKind::Entity));
                }
                u
            }
            _ => self.entity_unit(scope, &c.entity)?,
        };
        self.decls_in(scope, &c.decls);
        let Some(b) = &c.block else {
            return Err(self.error(
                Diagnostic::error(
                    diag::NOT_AN_INSTANCE,
                    "a configuration declaration needs a block configuration",
                )
                .with_primary(c.name.span, "no `for <architecture> ... end for;`"),
            ));
        };
        let config_regions = [self.units[unit].context, scope];
        self.arch_block_config(&config_regions, entity, b)
            .map(UnitHir::Configuration)
    }

    // -- instances --------------------------------------------------------------------

    /// The component, entity (with architecture) or configuration an instance names (11.7).
    pub(crate) fn inst_target(
        &mut self,
        scope: ScopeId,
        u: &ast::InstantiatedUnit,
    ) -> Result<InstTarget, Reported> {
        match u {
            ast::InstantiatedUnit::Component(n) => self.component_target(scope, n),
            ast::InstantiatedUnit::Entity { name, arch } => {
                let unit = self.entity_unit(scope, name)?;
                if let Some(a) = arch {
                    self.check_arch(unit, *a)?;
                }
                Ok(InstTarget::Entity { unit, arch: *arch })
            }
            ast::InstantiatedUnit::Configuration(n) => Ok(InstTarget::Configuration(
                self.configuration_unit(scope, n)?,
            )),
        }
    }

    /// The component a `[component] c` instance names, or `E1700`.
    pub(crate) fn component_target(
        &mut self,
        scope: ScopeId,
        n: &ast::Name,
    ) -> Result<InstTarget, Reported> {
        let d = self.expanded_name_decl(scope, n)?;
        let d = self.entity(d);
        match &self.decls[d].kind {
            DeclKind::Component(_) => {
                self.note_default_binding(scope, self.decls[d].name.key(), n.span);
                Ok(InstTarget::Component(d))
            }
            DeclKind::Error => Err(Reported),
            other => {
                let help = matches!(other, DeclKind::Unit(u) if matches!(self.units[*u].kind, UnitKind::Entity | UnitKind::Configuration));
                let mut diag = Diagnostic::error(
                    diag::NOT_INSTANTIABLE,
                    format!(
                        "`{}` is not a component, entity or configuration",
                        self.decls[d].name
                    ),
                )
                .with_primary(n.span, "cannot be instantiated")
                .with_secondary(self.decls[d].span, "declared here");
                if help {
                    diag = diag.with_help(
                                "instantiate an entity with `entity lib.name` and a configuration with `configuration lib.name`",
                            );
                }
                Err(self.error(diag))
            }
        }
    }

    /// Whether `l : n;` instantiates (a simple name that is not a subprogram)
    /// rather than calls a procedure.
    pub(crate) fn names_component(&mut self, scope: ScopeId, n: &ast::Name) -> bool {
        let ast::NameKind::Simple(i) = &n.kind else {
            return false;
        };
        match self.lookup(scope, i.key) {
            crate::scope::Visible::One(d) => !self.is_overloadable(d),
            _ => false,
        }
    }

    fn entity_unit(&mut self, scope: ScopeId, n: &ast::Name) -> Result<UnitId, Reported> {
        let d = self.expanded_name_decl(scope, n)?;
        match self.decls[d].kind {
            DeclKind::Unit(u) if self.units[u].kind == UnitKind::Entity => Ok(u),
            DeclKind::Unit(u) => Err(self.wrong_kind(u, n.span, UnitKind::Entity)),
            DeclKind::Error => Err(Reported),
            _ => Err(self.error(
                Diagnostic::error(
                    diag::NOT_INSTANTIABLE,
                    format!("`{}` is not an entity", self.decls[d].name),
                )
                .with_primary(n.span, "expected an entity"),
            )),
        }
    }

    fn configuration_unit(&mut self, scope: ScopeId, n: &ast::Name) -> Result<UnitId, Reported> {
        let d = self.expanded_name_decl(scope, n)?;
        match self.decls[d].kind {
            DeclKind::Unit(u) if self.units[u].kind == UnitKind::Configuration => Ok(u),
            DeclKind::Unit(u) => Err(self.wrong_kind(u, n.span, UnitKind::Configuration)),
            DeclKind::Error => Err(Reported),
            _ => Err(self.error(
                Diagnostic::error(
                    diag::NOT_INSTANTIABLE,
                    format!("`{}` is not a configuration", self.decls[d].name),
                )
                .with_primary(n.span, "expected a configuration"),
            )),
        }
    }

    /// Architecture `a` of entity `entity` must exist (it is not analyzed:
    /// an architecture may instantiate itself). Recorded as a dependency.
    fn check_arch(&mut self, entity: UnitId, a: Ident) -> Result<(), Reported> {
        let lib = self.units[entity].lib;
        let ekey = self.units[entity].name.key;
        let key = UnitKey::Secondary(ekey, a.key);
        if !self.libs[lib].units.contains_key(&key) && self.locate(lib, key).is_none() {
            let lname = self.libs[lib].name.text;
            return Err(self.error(
                Diagnostic::error(
                    diag::UNIT_NOT_FOUND,
                    format!(
                        "architecture `{}` of entity `{}` not found in library `{lname}`",
                        a.text, self.units[entity].name.text
                    ),
                )
                .with_primary(a.span, "no architecture with this name")
                .with_help("analyze the architecture first, or check the name"),
            ));
        }
        self.add_ref(lib, LibKind::Architecture, format!("{ekey}/{}", a.key));
        Ok(())
    }

    /// A manifest dependency of the unit being analyzed on a unit it does not load.
    fn add_ref(&mut self, lib: LibId, kind: LibKind, key: String) {
        if matches!(self.libs[lib].source, LibSource::StdDir(_)) {
            return;
        }
        let Some(c) = self.cur() else { return };
        let r = UnitRef {
            library: self.libs[lib].name.key.to_string(),
            kind,
            key,
        };
        if !self.units[c.unit].refs.contains(&r) {
            self.units[c.unit].refs.push(r);
        }
    }

    /// `l` in `architecture a of l.e` (3.3.1): a library named in the context
    /// clause (or `work`) that is the architecture's own library.
    pub(super) fn arch_entity_lib(
        &mut self,
        lib: LibId,
        l: Ident,
        entity: Ident,
        context: &[ast::ContextItem],
    ) -> Result<(), Reported> {
        let is_work = l.key.as_str() == "work";
        // ponytail: library clauses reached through `context c;` are not seen here.
        let named = context.iter().any(|c| {
            matches!(c, ast::ContextItem::Library { names, .. } if names.iter().any(|n| n.key == l.key))
        });
        if !is_work && !named {
            return Err(self.error(
                Diagnostic::error(diag::NOT_DECLARED, format!("`{}` is not declared", l.text))
                    .with_primary(l.span, "not a visible library name")
                    .with_help(format!("add `library {};` before the architecture", l.text)),
            ));
        }
        if is_work {
            return Ok(());
        }
        let pl = self.library(l)?;
        if pl == lib {
            return Ok(());
        }
        let lname = self.libs[pl].name.text;
        if !self.has_entity(pl, entity.key) {
            return Err(self.error(
                Diagnostic::error(
                    diag::UNIT_NOT_FOUND,
                    format!(
                        "design unit `{}` not found in library `{lname}`",
                        entity.text
                    ),
                )
                .with_primary(l.span.to(entity.span), "no entity with this name"),
            ));
        }
        let own = self.libs[lib].name.text;
        Err(self.error(
            Diagnostic::error(
                diag::NO_PRIMARY,
                format!(
                    "entity `{}` is in library `{lname}`, not in `{own}`",
                    entity.text
                ),
            )
            .with_primary(l.span.to(entity.span), "entity of another library")
            .with_note("an architecture and its entity reside in the same library (LRM 3.3.1)"),
        ))
    }

    /// Whether `lib` has an entity named `key`, without analyzing it.
    fn has_entity(&self, lib: LibId, key: Symbol) -> bool {
        let pk = UnitKey::Primary(key);
        match self.libs[lib].units.get(&pk) {
            Some(UnitSlot::Done(u) | UnitSlot::InProgress(u)) => {
                self.units[*u].kind == UnitKind::Entity
            }
            Some(UnitSlot::NotLoaded { file, .. }) => {
                self.files.get(file).and_then(|f| f.units.as_ref()).is_some_and(|us| {
                    us.iter().any(|(k, u)| {
                        *k == pk
                            && matches!(u, Some(du) if matches!(du.unit, ast::LibraryUnit::Entity(_)))
                    })
                })
            }
            Some(UnitSlot::Failed) => false,
            None => match &self.libs[lib].source {
                LibSource::Manifest(m) => m.find(LibKind::Entity, key.as_str()).is_some(),
                _ => false,
            },
        }
    }

    /// The library of the entity default binding picks for a component named
    /// `key` instantiated in `scope` (7.3.3, Q4): an entity made directly
    /// visible by a use clause, else the one in the library of the design unit.
    /// The second value is the `work` candidate when it differs (GHDL #66).
    fn default_entity_lib(&self, scope: ScopeId, key: Symbol) -> (Option<LibId>, Option<LibId>) {
        let mut visible = None;
        let mut cur = Some(scope);
        'walk: while let Some(s) = cur {
            for u in self.scopes[s].uses.iter().rev() {
                let found = match u {
                    UseItem::One(ds) => ds.iter().find_map(|&d| match self.decls[d].kind {
                        DeclKind::Unit(u)
                            if self.units[u].kind == UnitKind::Entity
                                && self.units[u].name.key == key =>
                        {
                            Some(self.units[u].lib)
                        }
                        _ => None,
                    }),
                    UseItem::Library(l) => self.has_entity(*l, key).then_some(*l),
                    UseItem::All(_) => None,
                };
                if found.is_some() {
                    visible = found;
                    break 'walk;
                }
            }
            cur = self.scopes[s].parent;
        }
        let work = self.unit_lib_of(scope).filter(|&l| self.has_entity(l, key));
        match visible {
            Some(v) => (Some(v), work.filter(|&w| w != v)),
            None => (work, None),
        }
    }

    /// The library of the design unit whose region contains `scope`.
    fn unit_lib_of(&self, scope: ScopeId) -> Option<LibId> {
        let mut cur = Some(scope);
        while let Some(s) = cur {
            if let Some(o) = self.scopes[s].owner
                && let DeclKind::Unit(u) = self.decls[o].kind
            {
                return Some(self.units[u].lib);
            }
            cur = self.scopes[s].parent;
        }
        None
    }

    /// Records the default-bound entity of a component instance as a
    /// dependency, and warns when two candidates exist (`W1703`).
    fn note_default_binding(&mut self, scope: ScopeId, key: Symbol, span: Span) {
        let (chosen, other) = self.default_entity_lib(scope, key);
        let Some(lib) = chosen else { return };
        self.add_ref(lib, LibKind::Entity, key.to_string());
        if let Some(w) = other {
            let (a, b) = (self.libs[lib].name.text, self.libs[w].name.text);
            self.report(
                Diagnostic::warning(
                    diag::W_DEFAULT_BINDING,
                    format!("default binding of `{key}` uses `{a}.{key}`, not `{b}.{key}`"),
                )
                .with_primary(span, "bound by default")
                .with_note(format!(
                    "`{a}.{key}` is directly visible through a use clause; `{b}` is the library of this design unit (LRM 7.3.3)"
                ))
                .with_help("bind it explicitly with a configuration specification to silence this"),
            );
        }
    }

    /// Default binding of component `comp` instantiated in `scope` (7.3.3, Q4):
    /// the entity and its most recently analyzed architecture, which with no
    /// cache is the last one in manifest (or analysis) order. For elab.
    pub fn default_binding(
        &mut self,
        scope: ScopeId,
        comp: DeclId,
    ) -> Option<(UnitId, Option<Symbol>)> {
        let key = self.decls[comp].name.key();
        let lib = self.default_entity_lib(scope, key).0?;
        let entity = self.load_unit(lib, UnitKey::Primary(key), None).ok()?;
        let arch = match &self.libs[lib].source {
            LibSource::Manifest(m) => m
                .units()
                .filter(|u| {
                    u.kind == LibKind::Architecture && u.primary.as_deref() == Some(key.as_str())
                })
                .last()
                .and_then(|u| u.key.split_once('/').map(|(_, a)| Symbol::intern(a))),
            _ => self
                .units
                .iter()
                .filter(|(_, u)| {
                    u.lib == lib
                        && u.kind == UnitKind::Architecture
                        && matches!(u.key, UnitKey::Secondary(e, _) if e == key)
                })
                .last()
                .map(|(_, u)| u.name.key),
        };
        Some((entity, arch))
    }

    // -- association lists ----------------------------------------------------------

    /// A generic or port map against `formals` (6.5.7): complete and in formal
    /// order. `at` is where missing actuals are reported.
    pub(crate) fn assoc_list(
        &mut self,
        scope: ScopeId,
        formals: &[DeclId],
        assocs: &[ast::AssocElem],
        at: Span,
    ) -> Result<Box<[Arg]>, Reported> {
        let mut errs = Errs::default();
        let mut map: Vec<Assoc> = formals.iter().map(|_| Assoc::None).collect();
        let mut named = false;
        for (k, a) in assocs.iter().enumerate() {
            let Some(f) = &a.formal else {
                if named {
                    errs.keep::<()>(Err(self.error(
                        Diagnostic::error(
                            diag::FORMAL_TWICE,
                            "positional association after named association",
                        )
                        .with_primary(a.span, "positional actual"),
                    )));
                    continue;
                }
                if k >= formals.len() {
                    errs.keep::<()>(Err(self.too_many(formals, a.span)));
                    break;
                }
                map[k] = Assoc::Whole {
                    idx: k,
                    conv: None,
                    span: a.span,
                };
                continue;
            };
            named = true;
            let Some((pos, conv, whole)) = errs.keep(self.formal_of(scope, formals, f)) else {
                continue;
            };
            let dup = match (&mut map[pos], whole) {
                (m @ Assoc::None, true) => {
                    *m = Assoc::Whole {
                        idx: k,
                        conv,
                        span: f.span,
                    };
                    false
                }
                (m @ Assoc::None, false) => {
                    *m = Assoc::Parts(vec![(k, f.span)]);
                    false
                }
                (Assoc::Parts(v), false) => {
                    v.push((k, f.span));
                    false
                }
                _ => true,
            };
            if dup {
                errs.keep::<()>(Err(self.error(
                    Diagnostic::error(
                        diag::FORMAL_TWICE,
                        format!(
                            "formal `{}` is associated twice",
                            self.decls[formals[pos]].name
                        ),
                    )
                    .with_primary(f.span, "associated again here"),
                )));
            }
        }
        let mut args = Vec::with_capacity(formals.len());
        for (k, m) in map.into_iter().enumerate() {
            let f = formals[k];
            let actual = match m {
                // After an association error a missing actual is likely a cascade.
                Assoc::None if errs.0 => None,
                Assoc::None => errs.keep(self.missing_actual(f, at)),
                Assoc::Whole { idx, conv, span } => {
                    errs.keep(self.actual(scope, f, &assocs[idx].actual, conv, span))
                }
                Assoc::Parts(parts) => errs.keep(self.partial(scope, f, assocs, &parts)),
            };
            if let Some(actual) = actual {
                args.push(Arg {
                    formal: k as u16,
                    actual,
                });
            }
        }
        errs.finish(args.into())
    }

    fn too_many(&mut self, formals: &[DeclId], span: Span) -> Reported {
        let (owner, what) = self.formal_owner(formals);
        self.error(
            Diagnostic::error(
                diag::NOT_DECLARED,
                format!("too many actuals: {owner} has {} {what}s", formals.len()),
            )
            .with_primary(span, "no formal for this actual"),
        )
    }

    /// `(entity `e`, "generic")` for messages about a formal list.
    fn formal_owner(&self, formals: &[DeclId]) -> (String, &'static str) {
        let Some(&f) = formals.first() else {
            return ("the unit".into(), "formal");
        };
        let what = match &self.decls[f].kind {
            DeclKind::Object(o) if o.role == ObjRole::Port => "port",
            _ => "generic",
        };
        let owner = self.scopes[self.decls[f].scope]
            .owner
            .map(|o| {
                let kind = match self.decls[o].kind {
                    DeclKind::Unit(_) => "entity",
                    DeclKind::Component(_) => "component",
                    _ => "block",
                };
                format!("{kind} `{}`", self.decls[o].name)
            })
            .unwrap_or_else(|| "the block".into());
        (owner, what)
    }

    /// The formal a named association designates: `(index, formal
    /// conversion, whole)`. `f`, `f(i)`, `f.e`, `f(1 to 3)` and `conv(f)`.
    fn formal_of(
        &mut self,
        scope: ScopeId,
        formals: &[DeclId],
        f: &ast::Name,
    ) -> Result<(usize, Option<DeclId>, bool), Reported> {
        let find =
            |db: &Db, key: Symbol| formals.iter().position(|&d| db.decls[d].name.key() == key);
        let root = root_ident(f);
        if let Some(r) = root
            && let Some(p) = find(self, r.key)
        {
            return Ok((p, None, matches!(f.kind, ast::NameKind::Simple(_))));
        }
        // `conv(formal) => actual` (6.5.7.1).
        if let ast::NameKind::Apply { prefix, args } = &f.kind
            && let [
                ast::AssocElem {
                    formal: None,
                    actual: ast::Actual::Expr(e),
                    ..
                },
            ] = &args[..]
            && let ast::ExprKind::Name(inner) = &e.kind
            && let ast::NameKind::Simple(i) = &inner.kind
            && let Some(p) = find(self, i.key)
        {
            let conv = self.formal_conversion(scope, prefix, formals[p])?;
            return Ok((p, Some(conv), true));
        }
        let (owner, what) = self.formal_owner(formals);
        let name = root.map_or_else(|| "?".to_string(), |r| r.text.to_string());
        let span = root.map_or(f.span, |r| r.span);
        let mut d = Diagnostic::error(
            diag::NOT_DECLARED,
            format!("{owner} has no {what} `{name}`"),
        )
        .with_primary(span, format!("not a {what} of {owner}"));
        let keys: Vec<String> = formals
            .iter()
            .map(|&d| self.decls[d].name.to_string())
            .collect();
        if let Some(best) = crate::suggest::closest(&name, keys.iter().map(String::as_str)) {
            d = d.with_help(format!("did you mean `{best}`?"));
        }
        Err(self.error(d))
    }

    /// The function or type mark `conv` in `conv(formal) => ...`: one
    /// parameter of the formal's type (4.2.2.? and 6.5.7.1).
    fn formal_conversion(
        &mut self,
        scope: ScopeId,
        conv: &ast::Name,
        formal: DeclId,
    ) -> Result<DeclId, Reported> {
        let ft = self.formal_subtype(formal).map(|st| self.base(st));
        let d = self.expanded_name_decl(scope, conv)?;
        if matches!(
            self.decls[self.entity(d)].kind,
            DeclKind::Type(_) | DeclKind::Subtype(_)
        ) {
            return Ok(self.entity(d));
        }
        let key = self.decls[d].name.key();
        let cands = match self.lookup(scope, key) {
            crate::scope::Visible::Overloads(ds) => ds,
            crate::scope::Visible::One(d) => vec![d],
            _ => Vec::new(),
        };
        let hit: Vec<DeclId> = cands
            .into_iter()
            .map(|d| self.entity(d))
            .filter(|&d| match self.decls[d].kind.subprogram() {
                Some(s) => {
                    s.ret.is_some()
                        && s.params.len() == 1
                        && Some(self.base(s.params[0].subtype)) == ft
                }
                None => false,
            })
            .collect();
        match hit[..] {
            [d] => Ok(d),
            _ => Err(self.error(
                Diagnostic::error(
                    diag::NO_OVERLOAD,
                    "no unique conversion function for this formal",
                )
                .with_primary(
                    conv.span,
                    "expected a function of one parameter of the formal's type",
                ),
            )),
        }
    }

    fn formal_subtype(&self, d: DeclId) -> Option<SubtypeId> {
        match &self.decls[d].kind {
            DeclKind::Object(o) => Some(o.subtype),
            _ => None,
        }
    }

    fn missing_actual(&mut self, f: DeclId, at: Span) -> Result<Actual, Reported> {
        let DeclKind::Object(o) = &self.decls[f].kind else {
            return Err(Reported);
        };
        if o.default.is_some() {
            return Ok(Actual::Default);
        }
        let name = self.decls[f].name;
        match (&o.role, o.mode) {
            (ObjRole::Port, Some(Mode::In)) => Err(self.error(
                Diagnostic::error(
                    diag::MISSING_ACTUAL,
                    format!("port `{name}` of mode `in` has no actual"),
                )
                .with_primary(at, format!("`{name}` is not associated"))
                .with_secondary(self.decls[f].span, "declared here without a default")
                .with_help(format!(
                    "associate it (`{name} => ...`) or give the port a default"
                )),
            )),
            (ObjRole::Port, _) => Ok(Actual::Open),
            _ => Err(self.error(
                Diagnostic::error(
                    diag::MISSING_GENERIC,
                    format!("generic `{name}` has no actual and no default"),
                )
                .with_primary(at, format!("`{name}` is not associated"))
                .with_secondary(self.decls[f].span, "declared here without a default")
                .with_help(format!("add `generic map ({name} => ...)`")),
            )),
        }
    }

    /// The actual of a whole formal.
    fn actual(
        &mut self,
        scope: ScopeId,
        f: DeclId,
        a: &ast::Actual,
        formal_conv: Option<DeclId>,
        fspan: Span,
    ) -> Result<Actual, Reported> {
        let DeclKind::Object(o) = &self.decls[f].kind else {
            return Err(Reported);
        };
        let (st, role, mode, has_default) =
            (o.subtype, o.role.clone(), o.mode, o.default.is_some());
        let e = match a {
            ast::Actual::Open(sp) => {
                if role == ObjRole::Port && mode == Some(Mode::In) && !has_default {
                    let name = self.decls[f].name;
                    return Err(self.error(
                        Diagnostic::error(
                            diag::MISSING_ACTUAL,
                            format!(
                                "port `{name}` of mode `in` without a default cannot be `open`"
                            ),
                        )
                        .with_primary(*sp, "left open here"),
                    ));
                }
                if role != ObjRole::Port && !has_default {
                    let name = self.decls[f].name;
                    return Err(self.error(
                        Diagnostic::error(
                            diag::MISSING_GENERIC,
                            format!("generic `{name}` has no default, so it cannot be `open`"),
                        )
                        .with_primary(*sp, "left open here"),
                    ));
                }
                return Ok(if role == ObjRole::Port {
                    Actual::Open
                } else {
                    Actual::Default
                });
            }
            ast::Actual::Expr(e) | ast::Actual::Inertial(e) => e,
            ast::Actual::Subtype(si) => {
                return Err(self.unsupported(si.span, "subtype actuals", "S4"));
            }
            ast::Actual::Range(_) => {
                return Err(self.unsupported(fspan, "range actuals", "S4"));
            }
        };
        // With a formal conversion the actual has the conversion's result type.
        let target = match formal_conv {
            Some(c) => match &self.decls[c].kind {
                DeclKind::Type(t) => self.types[*t].first_subtype,
                DeclKind::Subtype(s) => *s,
                k => k.subprogram().and_then(|s| s.ret).unwrap_or(st),
            },
            None => st,
        };
        let id = self.expr_in_subtype(scope, e, target);
        if self.is_poison_expr(id) {
            return Err(Reported);
        }
        if role != ObjRole::Port {
            return Ok(Actual::Expr(id));
        }
        // Port actuals (6.5.6.3, 6.5.7.1): a conversion of a signal, or for
        // modes other than `in` a signal name.
        let actual_conv = match &self.exprs[id].kind {
            ExprKind::Call { callee, args, .. } if args.len() == 1 => match &args[0].actual {
                Actual::Expr(x) if self.is_signal_name(*x) => Some((Some(*callee), *x)),
                _ => None,
            },
            ExprKind::Conv { to, operand } if self.is_signal_name(*operand) => {
                let t = self.subtypes[*to].base;
                Some((Some(self.types[t].decl), *operand))
            }
            _ => None,
        };
        if let Some((conv, x)) = actual_conv
            && mode != Some(Mode::Out)
            && mode != Some(Mode::Buffer)
        {
            return Ok(Actual::Converted {
                formal_conv,
                actual_conv: conv,
                actual: x,
            });
        }
        if mode != Some(Mode::In) && !self.is_signal_name(id) {
            let what = match self.lsp(id).map(|l| l.root) {
                Some(ObjRoot::Decl(d)) => match &self.decls[d].kind {
                    DeclKind::Object(o) => {
                        format!("`{}` is a {}", self.decls[d].name, class_str(o.class))
                    }
                    _ => "this is not a signal".into(),
                },
                _ => "this expression is not a signal name".into(),
            };
            let m = mode_str(mode);
            return Err(self.error(
                Diagnostic::error(
                    diag::CLASS_MISUSE,
                    format!("{what} and cannot be an actual for an `{m}` port"),
                )
                .with_primary(self.exprs[id].span, "not a signal")
                .with_help(format!(
                    "a port of mode `{m}` needs a signal (or `open`) as its actual"
                )),
            ));
        }
        Ok(match formal_conv {
            Some(_) => Actual::Converted {
                formal_conv,
                actual_conv: None,
                actual: id,
            },
            None => Actual::Expr(id),
        })
    }

    fn is_signal_name(&self, e: ExprId) -> bool {
        match self.lsp(e).map(|l| l.root) {
            Some(r @ ObjRoot::Decl(_)) | Some(r @ ObjRoot::External(_)) => self.is_signal(r),
            Some(ObjRoot::ImplicitSignal(_)) => true,
            None => false,
        }
    }

    /// Sub-element associations of one formal (6.5.7.1): `f(0) => a, f(1) => b`.
    fn partial(
        &mut self,
        scope: ScopeId,
        f: DeclId,
        assocs: &[ast::AssocElem],
        parts: &[(usize, Span)],
    ) -> Result<Actual, Reported> {
        let fscope = self.decls[f].scope;
        let mut errs = Errs::default();
        let mut out = Vec::new();
        for &(k, span) in parts {
            let a = &assocs[k];
            let Some(fname) = &a.formal else { continue };
            let saved = std::mem::take(&mut self.memo);
            let cs = self.name_cands(fscope, fname);
            let fe = match cs {
                crate::expr::CandSet::Types(is) if is.len() == 1 => {
                    Some(self.build_name(fscope, fname, is[0], span))
                }
                _ => None,
            };
            self.memo = saved;
            let Some(fe) = fe.filter(|&e| !self.is_poison_expr(e)) else {
                errs.keep::<()>(Err(self.error(
                    Diagnostic::error(diag::NOT_AN_OBJECT, "not a sub-element of the formal")
                        .with_primary(
                            span,
                            "expected `formal(index)`, `formal(range)` or `formal.element`",
                        ),
                )));
                continue;
            };
            let Some((root, steps, stat, _)) = self.lsp_parts(fe) else {
                errs.keep::<()>(Err(self.ice(span, "formal sub-element without a prefix")));
                continue;
            };
            if root != ObjRoot::Decl(f) || stat < crate::stat::Staticness::Locally {
                errs.keep::<()>(Err(self.error(
                    Diagnostic::error(
                        diag::NOT_STATIC,
                        "a formal sub-element must be a locally static name",
                    )
                    .with_primary(span, "not locally static"),
                )));
                continue;
            }
            let (ast::Actual::Expr(e) | ast::Actual::Inertial(e)) = &a.actual else {
                errs.keep::<()>(Err(self.unsupported(
                    span,
                    "`open` or range actuals of sub-elements",
                    "S5",
                )));
                continue;
            };
            let st = self.exprs[fe].subtype;
            let id = self.expr_in_subtype(scope, e, st);
            if self.is_poison_expr(id) {
                errs.0 = true;
                continue;
            }
            out.push((steps.into_boxed_slice(), id));
        }
        errs.finish(Actual::Individual(out.into()))
    }

    // -- configuration specifications and declarations -------------------------------

    /// `for l : c use ...;` in a declarative part (7.3.1). Labels are resolved
    /// after the statements of the region (`resolve_pending_specs`).
    pub(crate) fn config_spec(
        &mut self,
        scope: ScopeId,
        c: &ast::ConfigSpec,
    ) -> Result<(), Reported> {
        let comp = self.component_of_spec(scope, &c.spec)?;
        if c.binding.entity_aspect.is_none() {
            return Err(self.error(
                Diagnostic::error(
                    diag::NO_ENTITY_BOUND,
                    "a configuration specification needs an entity aspect",
                )
                .with_primary(c.binding.span, "no `use entity`, `use configuration` or `use open`")
                .with_note("a binding indication in a configuration specification is primary (LRM 7.3.2.1)"),
            ));
        }
        let binding = self.binding_indication(scope, comp, &c.binding, None)?;
        let Some(cx) = self.cur() else {
            return Err(self.ice(c.span, "configuration specification outside a design unit"));
        };
        self.pending_specs.push(PendingSpec {
            unit: cx.unit,
            scope,
            labels: Labels::of(&c.spec.instances),
            component: comp,
            binding,
            span: c.span,
        });
        Ok(())
    }

    fn component_of_spec(
        &mut self,
        scope: ScopeId,
        s: &ast::ComponentSpec,
    ) -> Result<DeclId, Reported> {
        let d = self.expanded_name_decl(scope, &s.component)?;
        let d = self.entity(d);
        match self.decls[d].kind {
            DeclKind::Component(_) => Ok(d),
            DeclKind::Error => Err(Reported),
            _ => Err(self.error(
                Diagnostic::error(
                    diag::NOT_INSTANTIABLE,
                    format!("`{}` is not a component", self.decls[d].name),
                )
                .with_primary(s.component.span, "expected a component name"),
            )),
        }
    }

    /// The instance labels of the specifications of `unit`, now that its
    /// statements are analyzed.
    fn resolve_pending_specs(&mut self, unit: UnitId) {
        let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending_specs)
            .into_iter()
            .partition(|p| p.unit == unit);
        self.pending_specs = rest;
        for p in mine {
            let Ok(instances) = self.instance_sel(&[p.scope], &p.labels, p.component, p.scope)
            else {
                continue;
            };
            self.config_specs
                .entry(p.scope)
                .or_default()
                .push(ComponentConfig {
                    instances,
                    component: p.component,
                    binding: Some(p.binding),
                    block: None,
                    span: p.span,
                });
        }
    }

    /// The instances a component specification names: each label must be an
    /// instance of `comp` in one of `regions` (E1701).
    fn instance_sel(
        &mut self,
        regions: &[ScopeId],
        labels: &Labels,
        comp: DeclId,
        named_in: ScopeId,
    ) -> Result<InstanceSel, Reported> {
        let ls = match labels {
            Labels::All => return Ok(InstanceSel::All),
            Labels::Others => return Ok(InstanceSel::Others),
            Labels::Some(ls) => ls,
        };
        let mut errs = Errs::default();
        let mut out = Vec::new();
        for l in ls {
            let stmt = regions.iter().find_map(|&r| {
                self.scopes[r].names.get(&l.key).and_then(|ds| {
                    ds.iter().find_map(|&d| match self.decls[d].kind {
                        DeclKind::Label(s) => Some(s),
                        _ => None,
                    })
                })
            });
            let region = self.region_name(named_in);
            match stmt.map(|s| (s, &self.stmts[s].kind)) {
                Some((
                    s,
                    StmtKind::Instance {
                        target: InstTarget::Component(c),
                        ..
                    },
                )) => {
                    if *c == comp {
                        out.push(s);
                    } else {
                        let (cn, want) = (self.decls[*c].name, self.decls[comp].name);
                        errs.keep::<()>(Err(self.error(
                            Diagnostic::error(
                                diag::NOT_AN_INSTANCE,
                                format!(
                                    "instance `{}` is of component `{cn}`, not `{want}`",
                                    l.text
                                ),
                            )
                            .with_primary(l.span, format!("an instance of `{cn}`"))
                            .with_secondary(self.stmts[s].span, "instantiated here"),
                        )));
                    }
                }
                _ => {
                    errs.keep::<()>(Err(self.error(
                        Diagnostic::error(
                            diag::NOT_AN_INSTANCE,
                            format!("`{}` is not a component instance in `{region}`", l.text),
                        )
                        .with_primary(l.span, "no component instance with this label"),
                    )));
                }
            }
        }
        errs.finish(InstanceSel::Labels(out.into()))
    }

    fn region_name(&self, s: ScopeId) -> String {
        self.scopes[s]
            .owner
            .map(|o| match self.decls[o].kind {
                DeclKind::Unit(u) => self.units[u].name.text.to_string(),
                _ => self.decls[o].name.to_string(),
            })
            .unwrap_or_default()
    }

    /// `use entity l.e(a) generic map (...) port map (...)` (7.3.2). Names are
    /// resolved in `scope`; actuals see the component's generics and ports.
    /// `bound`: the entity an earlier specification bound the instances to.
    fn binding_indication(
        &mut self,
        scope: ScopeId,
        comp: DeclId,
        b: &ast::BindingIndication,
        bound: Option<UnitId>,
    ) -> Result<Binding, Reported> {
        let (aspect, entity, at) = match &b.entity_aspect {
            Some(ast::EntityAspect::Entity { name, arch }) => {
                let u = self.entity_unit(scope, name)?;
                if let Some(a) = arch {
                    self.check_arch(u, *a)?;
                }
                (
                    Some(EntityAspect::Entity {
                        unit: u,
                        arch: *arch,
                    }),
                    Some(u),
                    name.span,
                )
            }
            Some(ast::EntityAspect::Configuration(n)) => {
                let u = self.configuration_unit(scope, n)?;
                let e = self.config_entity(u);
                (Some(EntityAspect::Configuration(u)), e, n.span)
            }
            Some(ast::EntityAspect::Open) => (Some(EntityAspect::Open), None, b.span),
            None => (None, bound, b.span),
        };
        let DeclKind::Component(c) = &self.decls[comp].kind else {
            return Err(Reported);
        };
        let (cscope, locals_g, locals_p) = (c.scope, c.generics.clone(), c.ports.clone());
        let Some(e) = entity else {
            if b.generic_map.is_some() || b.port_map.is_some() {
                if matches!(aspect, Some(EntityAspect::Open)) {
                    return Err(self.error(
                        Diagnostic::error(
                            diag::BINDING_MISMATCH,
                            "`use open` takes no generic or port map",
                        )
                        .with_primary(b.span, "maps given here"),
                    ));
                }
                return Err(self.error(
                    Diagnostic::error(
                        diag::NO_ENTITY_BOUND,
                        "this binding adds maps, but no configuration specification binds the instance to an entity",
                    )
                    .with_primary(b.span, "no primary binding")
                    .with_note("an incremental binding indication refines a primary binding (LRM 7.3.2.1)")
                    .with_help("add `use entity lib.name` here, or bind it with a configuration specification"),
                ));
            }
            return Ok(Binding {
                aspect,
                generic_map: Box::default(),
                port_map: Box::default(),
            });
        };
        let UnitHir::Entity {
            generics, ports, ..
        } = &self.units[e].hir
        else {
            return Err(Reported);
        };
        let (eg, ep) = (generics.to_vec(), ports.to_vec());
        let mut errs = Errs::default();
        let generic_map = match &b.generic_map {
            Some(m) => errs.keep(self.assoc_list(cscope, &eg, m, b.span)),
            None => errs.keep(self.default_map(&locals_g, &eg, e, at)),
        };
        let port_map = match &b.port_map {
            Some(m) => errs.keep(self.assoc_list(cscope, &ep, m, b.span)),
            None => errs.keep(self.default_map(&locals_p, &ep, e, at)),
        };
        errs.finish(())?;
        Ok(Binding {
            aspect,
            generic_map: generic_map.unwrap_or_default(),
            port_map: port_map.unwrap_or_default(),
        })
    }

    /// The default generic or port map of a binding (7.3.3): each entity formal
    /// gets the component local of the same name; a local without such a
    /// formal, or of another type, is `E1703`.
    fn default_map(
        &mut self,
        locals: &[DeclId],
        formals: &[DeclId],
        entity: UnitId,
        at: Span,
    ) -> Result<Box<[Arg]>, Reported> {
        let mut errs = Errs::default();
        for &l in locals {
            let key = self.decls[l].name.key();
            let hit = formals.iter().find(|&&f| self.decls[f].name.key() == key);
            let ok = hit.is_some_and(
                |&f| match (self.formal_subtype(l), self.formal_subtype(f)) {
                    (Some(a), Some(b)) => self.base(a) == self.base(b),
                    _ => false,
                },
            );
            if !ok {
                let what = match &self.decls[l].kind {
                    DeclKind::Object(o) if o.role == ObjRole::Port => "port",
                    _ => "generic",
                };
                let (ln, en) = (self.decls[l].name, self.units[entity].name.text);
                let msg = if hit.is_some() {
                    format!(
                        "component {what} `{ln}` and the {what} of entity `{en}` have different types"
                    )
                } else {
                    format!("component {what} `{ln}` has no matching {what} in entity `{en}`")
                };
                errs.keep::<()>(Err(self.error(
                    Diagnostic::error(diag::BINDING_MISMATCH, msg)
                        .with_primary(at, "bound here")
                        .with_secondary(self.decls[l].span, format!("component {what}"))
                        .with_help(format!("map it explicitly with a `{what} map`")),
                )));
            }
        }
        for &f in formals {
            let key = self.decls[f].name.key();
            let DeclKind::Object(o) = &self.decls[f].kind else {
                continue;
            };
            let unconnected_in = o.role == ObjRole::Port
                && o.mode == Some(Mode::In)
                && o.default.is_none()
                && !locals.iter().any(|&l| self.decls[l].name.key() == key);
            if unconnected_in {
                let (fname, en) = (self.decls[f].name, self.units[entity].name.text);
                errs.keep::<()>(Err(self.error(
                    Diagnostic::error(
                        diag::BINDING_MISMATCH,
                        format!("port `{fname}` of entity `{en}` has no default and no matching component port"),
                    )
                    .with_primary(at, "bound here")
                    .with_secondary(self.decls[f].span, "entity port of mode `in` without a default")
                    .with_help("map it explicitly with a `port map`"),
                )));
            }
        }
        errs.finish(())?;
        let mut args = Vec::with_capacity(formals.len());
        for (k, &f) in formals.iter().enumerate() {
            let key = self.decls[f].name.key();
            let actual = match locals.iter().find(|&&l| self.decls[l].name.key() == key) {
                Some(&l) => Actual::Expr(self.object_ref(l, at)),
                None => match &self.decls[f].kind {
                    DeclKind::Object(o) if o.default.is_none() && o.role == ObjRole::Port => {
                        Actual::Open
                    }
                    _ => Actual::Default,
                },
            };
            args.push(Arg {
                formal: k as u16,
                actual,
            });
        }
        Ok(args.into())
    }

    fn config_entity(&self, cfg: UnitId) -> Option<UnitId> {
        match &self.units[cfg].hir {
            UnitHir::Configuration(b) => match b.target {
                BlockTarget::Architecture(a) => match &self.units[a].hir {
                    UnitHir::Architecture { entity, .. } => Some(*entity),
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        }
    }

    /// `for arch ... end for;` of entity `entity` (3.4.2). `extra`: regions
    /// whose names and use clauses the configuration items also see (the
    /// configuration's context and declarations).
    fn arch_block_config(
        &mut self,
        extra: &[ScopeId],
        entity: UnitId,
        b: &ast::BlockConfig,
    ) -> Result<BlockConfig, Reported> {
        let ast::NameKind::Simple(a) = &b.spec.kind else {
            return Err(self.error(
                Diagnostic::error(
                    diag::NOT_AN_INSTANCE,
                    "expected the name of an architecture",
                )
                .with_primary(b.spec.span, "not an architecture name"),
            ));
        };
        let lib = self.units[entity].lib;
        let ekey = self.units[entity].name.key;
        let key = UnitKey::Secondary(ekey, a.key);
        if !self.libs[lib].units.contains_key(&key) && self.locate(lib, key).is_none() {
            return Err(self.error(
                Diagnostic::error(
                    diag::UNIT_NOT_FOUND,
                    format!(
                        "architecture `{}` of entity `{}` not found",
                        a.text, self.units[entity].name.text
                    ),
                )
                .with_primary(a.span, "no architecture with this name"),
            ));
        }
        let arch = self.load_dep(lib, key, Some(a.span))?;
        let ascope = self.units[arch].scope;
        let cscope = self.add_scope(ScopeKind::Configuration, Some(ascope), None);
        for &r in extra {
            let names: Vec<(Symbol, Vec<DeclId>)> = self.scopes[r]
                .names
                .iter()
                .map(|(k, v)| (*k, v.clone()))
                .collect();
            for (k, ds) in names {
                self.scopes[cscope].names.entry(k).or_default().extend(ds);
            }
            let uses = self.scopes[r].uses.clone();
            self.scopes[cscope].uses.extend(uses);
        }
        for u in &b.uses {
            self.use_clause(cscope, u);
        }
        let items = self.config_items(cscope, &[ascope], ascope, &b.items)?;
        Ok(BlockConfig {
            target: BlockTarget::Architecture(arch),
            items,
            span: b.span,
        })
    }

    fn config_items(
        &mut self,
        scope: ScopeId,
        regions: &[ScopeId],
        named_in: ScopeId,
        items: &[ast::ConfigItem],
    ) -> Result<Vec<ConfigItem>, Reported> {
        let mut errs = Errs::default();
        let mut out = Vec::new();
        for it in items {
            match it {
                ast::ConfigItem::Block(b) => {
                    if let Some(b) = errs.keep(self.inner_block_config(scope, regions, named_in, b))
                    {
                        out.push(ConfigItem::Block(b));
                    }
                }
                ast::ConfigItem::Component(c) => {
                    if let Some(c) = errs.keep(self.component_config(scope, regions, named_in, c)) {
                        out.push(ConfigItem::Component(c));
                    }
                }
            }
        }
        errs.finish(out)
    }

    /// `for block_label` / `for gen_label(i)` / `for gen_label(alt)` (3.4.2).
    fn inner_block_config(
        &mut self,
        scope: ScopeId,
        regions: &[ScopeId],
        named_in: ScopeId,
        b: &ast::BlockConfig,
    ) -> Result<BlockConfig, Reported> {
        let (label, arg) = match &b.spec.kind {
            ast::NameKind::Simple(i) => (*i, None),
            ast::NameKind::Apply { prefix, args } => match (&prefix.kind, &args[..]) {
                (ast::NameKind::Simple(i), [a]) if a.formal.is_none() => (*i, Some(&a.actual)),
                _ => return Err(self.not_block(b.spec.span, named_in)),
            },
            _ => return Err(self.not_block(b.spec.span, named_in)),
        };
        let stmt = regions.iter().find_map(|&r| {
            self.scopes[r].names.get(&label.key).and_then(|ds| {
                ds.iter().find_map(|&d| match self.decls[d].kind {
                    DeclKind::Label(s) => Some(s),
                    _ => None,
                })
            })
        });
        let Some(stmt) = stmt else {
            return Err(self.not_block(label.span, named_in));
        };
        let (target, inner): (BlockTarget, Vec<ScopeId>) = match &self.stmts[stmt].kind {
            StmtKind::Block { scope: s, .. } if arg.is_none() => {
                (BlockTarget::Block(stmt), vec![*s])
            }
            StmtKind::ForGenerate {
                param, scope: s, ..
            } => {
                let s = *s;
                let select = match arg {
                    None => None,
                    Some(ast::Actual::Expr(e)) => {
                        let st = self.formal_subtype(*param).unwrap_or(self.poison.subtype);
                        let id = self.expr_in_subtype(scope, e, st);
                        if self.is_poison_expr(id) {
                            return Err(Reported);
                        }
                        Some(GenSelect::Index(id))
                    }
                    Some(ast::Actual::Range(r)) => {
                        let st = self.formal_subtype(*param).unwrap_or(self.poison.subtype);
                        let base = self.base(st);
                        Some(GenSelect::Range(self.range(scope, r, base)?))
                    }
                    Some(ast::Actual::Subtype(si)) => {
                        // ponytail: `for g(natural range 1 to 3)`; add with a RangeC of a subtype.
                        return Err(self.unsupported(
                            si.span,
                            "subtype indications as generate specifications",
                            "S5",
                        ));
                    }
                    Some(_) => return Err(self.not_block(b.spec.span, named_in)),
                };
                (BlockTarget::Generate { stmt, select }, vec![s])
            }
            StmtKind::IfGenerate { alts } => {
                let scopes: Vec<ScopeId> = alts.iter().map(|a| a.scope).collect();
                let select = match arg {
                    Some(ast::Actual::Expr(ast::Expr {
                        kind:
                            ast::ExprKind::Name(ast::Name {
                                kind: ast::NameKind::Simple(i),
                                ..
                            }),
                        ..
                    })) => Some(GenSelect::Alternative(i.key)),
                    None => None,
                    Some(_) => return Err(self.not_block(b.spec.span, named_in)),
                };
                (BlockTarget::Generate { stmt, select }, scopes)
            }
            StmtKind::CaseGenerate { alts, .. } => {
                let scopes: Vec<ScopeId> = alts.iter().map(|(_, a)| a.scope).collect();
                let select = match arg {
                    Some(ast::Actual::Expr(ast::Expr {
                        kind:
                            ast::ExprKind::Name(ast::Name {
                                kind: ast::NameKind::Simple(i),
                                ..
                            }),
                        ..
                    })) => Some(GenSelect::Alternative(i.key)),
                    None => None,
                    Some(_) => return Err(self.not_block(b.spec.span, named_in)),
                };
                (BlockTarget::Generate { stmt, select }, scopes)
            }
            _ => return Err(self.not_block(label.span, named_in)),
        };
        let bscope = self.add_scope(ScopeKind::Configuration, Some(scope), None);
        for u in &b.uses {
            self.use_clause(bscope, u);
        }
        let first = inner[0];
        let items = self.config_items(bscope, &inner, first, &b.items)?;
        Ok(BlockConfig {
            target,
            items,
            span: b.span,
        })
    }

    fn not_block(&mut self, span: Span, named_in: ScopeId) -> Reported {
        let region = self.region_name(named_in);
        self.error(
            Diagnostic::error(
                diag::NOT_AN_INSTANCE,
                format!("not a block or generate statement in `{region}`"),
            )
            .with_primary(span, "expected the label of a block or generate statement"),
        )
    }

    /// `for l : c [binding] [block configuration] end for;` (3.4.3).
    fn component_config(
        &mut self,
        scope: ScopeId,
        regions: &[ScopeId],
        named_in: ScopeId,
        c: &ast::ComponentConfig,
    ) -> Result<ComponentConfig, Reported> {
        let comp = self.component_of_spec(scope, &c.spec)?;
        let instances =
            self.instance_sel(regions, &Labels::of(&c.spec.instances), comp, named_in)?;
        // An earlier configuration specification of the same instances (7.3.2.?):
        // incremental binding may add maps but not rebind.
        let spec = self.spec_for(regions, &instances, comp);
        let bound = spec.as_ref().and_then(|a| match a {
            Some(EntityAspect::Entity { unit, .. }) => Some(*unit),
            _ => None,
        });
        let binding = match &c.binding {
            Some(b) => {
                if b.entity_aspect.is_some() && matches!(spec, Some(Some(_))) {
                    let what = match &instances {
                        InstanceSel::Labels(ls) if ls.len() == 1 => {
                            self.stmts[ls[0]].label.map_or_else(
                                || "this instance".into(),
                                |l| format!("instance `{}`", l.text),
                            )
                        }
                        _ => "these instances".into(),
                    };
                    return Err(self.error(
                        Diagnostic::error(
                            diag::REBINDING,
                            format!("{what} is already bound by a configuration specification"),
                        )
                        .with_primary(b.span, "bound again here")
                        .with_help("an incremental binding may only add generic and port maps: drop the entity aspect"),
                    ));
                }
                Some(self.binding_indication(scope, comp, b, bound)?)
            }
            None => None,
        };
        let block = match &c.block {
            Some(bc) => {
                let e = match binding.as_ref().and_then(|b| b.aspect.as_ref()) {
                    Some(EntityAspect::Entity { unit, .. }) => Some(*unit),
                    _ => bound,
                };
                // Without an explicit binding the default binding applies (3.4.3, 7.3.3);
                // NVC and GHDL reject this, Questa accepts (permissive, decisions.md).
                let e = match e {
                    Some(e) => Some(e),
                    None if binding.is_none() => {
                        let region = regions.first().copied().unwrap_or(scope);
                        self.default_binding(region, comp).map(|(e, _)| e)
                    }
                    None => None,
                };
                let Some(e) = e else {
                    let cn = self.decls[comp].name;
                    return Err(self.error(
                        Diagnostic::error(
                            diag::NO_ENTITY_BOUND,
                            format!("no entity is bound to component `{cn}`, so it has no architecture to configure"),
                        )
                        .with_primary(bc.span, "block configuration of an unbound instance")
                        .with_help(format!("bind it with `use entity lib.{cn}` or analyze an entity `{cn}`")),
                    ));
                };
                Some(self.arch_block_config(&[scope], e, bc)?)
            }
            None => None,
        };
        Ok(ComponentConfig {
            instances,
            component: comp,
            binding,
            block,
            span: c.span,
        })
    }

    /// The entity aspect of a configuration specification that covers one of
    /// `sel`'s instances: `Some(None)` for a specification without one.
    fn spec_for(
        &self,
        regions: &[ScopeId],
        sel: &InstanceSel,
        comp: DeclId,
    ) -> Option<Option<EntityAspect>> {
        for r in regions {
            for s in self.config_specs.get(r).into_iter().flatten() {
                if s.component != comp {
                    continue;
                }
                let covers = match (&s.instances, sel) {
                    (InstanceSel::Labels(a), InstanceSel::Labels(b)) => {
                        a.iter().any(|x| b.contains(x))
                    }
                    _ => true,
                };
                if covers {
                    let aspect = s.binding.as_ref().and_then(|b| match &b.aspect {
                        Some(EntityAspect::Entity { unit, arch }) => Some(EntityAspect::Entity {
                            unit: *unit,
                            arch: *arch,
                        }),
                        Some(EntityAspect::Configuration(u)) => {
                            Some(EntityAspect::Configuration(*u))
                        }
                        Some(EntityAspect::Open) => Some(EntityAspect::Open),
                        None => None,
                    });
                    return Some(aspect);
                }
            }
        }
        None
    }
}

/// The leftmost simple name of `f`, `f(i)`, `f.e`, `f(i).e`.
fn root_ident(n: &ast::Name) -> Option<Ident> {
    match &n.kind {
        ast::NameKind::Simple(i) => Some(*i),
        ast::NameKind::Apply { prefix, .. } | ast::NameKind::Selected { prefix, .. } => {
            root_ident(prefix)
        }
        _ => None,
    }
}

fn class_str(c: ObjClass) -> &'static str {
    match c {
        ObjClass::Constant => "constant",
        ObjClass::Signal => "signal",
        ObjClass::Variable | ObjClass::SharedVariable => "variable",
        ObjClass::File => "file",
    }
}

fn mode_str(m: Option<Mode>) -> &'static str {
    match m {
        Some(Mode::Out) => "out",
        Some(Mode::Inout) => "inout",
        Some(Mode::Buffer) => "buffer",
        Some(Mode::Linkage) => "linkage",
        _ => "in",
    }
}

use crate::ids::ExprId;
