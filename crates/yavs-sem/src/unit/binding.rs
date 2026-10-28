//! Entities, architectures, components, instances, generic and port maps,
//! configuration specifications and declarations, default binding
//! (sem.md 6.5, 7.2; LRM 3, 6.5.6, 6.5.7, 7.3, 11.7).

use yavs_diag::{Diagnostic, Span};
use yavs_lib::{UnitKind as LibKind, UnitRef};
use yavs_syntax::Symbol;
use yavs_syntax::ast::{self, Ident};

use crate::db::{Db, Reported};
use crate::decl::{DeclKind, ObjClass, ObjRole};
use crate::diag;
use crate::hir::{
    Actual, Arg, Binding, BlockConfig, BlockTarget, ComponentConfig, ConfigItem, EntityAspect,
    ExprKind, GenSelect, InstTarget, InstanceSel, Mode, ObjRoot, StmtKind, UnitHir,
};
use crate::ids::{DeclId, LibId, ScopeId, StmtId, SubtypeId, TypeId, UnitId};
use crate::loader::{LibSource, UnitKey, UnitKind, UnitSlot};
use crate::scope::{ScopeKind, UseItem, Visible};

/// A configuration specification whose instance labels are resolved once
/// the statement part of its region has been analyzed.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct PendingSpec {
    unit: UnitId,
    /// The region the specification is declared in (its labels are declared there).
    scope: ScopeId,
    labels: Labels,
    component: DeclId,
    binding: Binding,
    span: Span,
}

#[derive(serde::Serialize, serde::Deserialize)]
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
        self.hidden_types.clear();
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
        // The anonymous interface types of generics and ports are generic types after the
        // declared generics, bound from the actuals of their objects (#283).
        let mut generics: Vec<DeclId> = generics.into_vec();
        generics.extend(self.take_hidden_types());
        let generics: Box<[DeclId]> = generics.into();
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
        self.instantiate_blocks(unit);
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
        // 3.4.1: a configuration and its entity reside in one library; 2019 relaxes it
        // (LCS-2016-023).
        if self.units[entity].lib != lib && self.file_rev() < yavs_syntax::Std::Vhdl2019 {
            let (own, other) = (
                self.libs[lib].name.text,
                self.libs[self.units[entity].lib].name.text,
            );
            return Err(self.error(
                Diagnostic::error(
                    diag::NOT_IN_REVISION,
                    format!(
                        "configuration `{}` is in library `{own}`, its entity `{}` in `{other}`",
                        c.name.text, self.units[entity].name.text
                    ),
                )
                .with_primary(c.entity.span, "entity of another library")
                .with_note("a configuration declaration and its entity reside in the same library before VHDL-2019 (LRM 3.4.1)"),
            ));
        }
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
                self.note_default_binding(scope, d, n.span);
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
        if !self.unit_known(lib, key) {
            let lname = self.libs[lib].name.text;
            let names = self.unit_names(lib, Some(ekey));
            let hint = crate::suggest::suggest(
                a.key.as_str(),
                "architecture",
                names.iter().map(|n| (n.as_str(), n.as_str())),
            );
            return Err(self.error(
                Diagnostic::error(
                    diag::UNIT_NOT_FOUND,
                    format!(
                        "architecture `{}` of entity `{}` not found in library `{lname}`",
                        a.text, self.units[entity].name.text
                    ),
                )
                .with_primary(a.span, "no architecture with this name")
                .with_help(
                    hint.unwrap_or_else(|| {
                        "analyze the architecture first, or check the name".into()
                    }),
                ),
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
        scope: Option<ScopeId>,
    ) -> Result<(), Reported> {
        let is_work = l.key.as_str() == "work";
        let mut named = context.iter().any(|c| {
            matches!(c, ast::ContextItem::Library { names, .. } if names.iter().any(|n| n.key == l.key))
        });
        // Library clauses spliced in by `context c;` are in the scope (13.4).
        if let Some(s) = scope {
            named |= matches!(
                self.lookup(s, l.key),
                Visible::One(d) if matches!(self.decls[d].kind, DeclKind::Library(_))
            );
        } else if context
            .iter()
            .any(|c| matches!(c, ast::ContextItem::Context { .. }))
        {
            return Ok(());
        }
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
        let a = self.has_entity_now(lib, key);
        self.note_query(
            crate::units::Query::Entity(self.libs[lib].name.key, key),
            crate::units::Answer::Bool(a),
        );
        a
    }

    pub(crate) fn has_entity_now(&self, lib: LibId, key: Symbol) -> bool {
        let pk = UnitKey::Primary(key);
        match self.libs[lib].units.get(&pk) {
            Some(UnitSlot::Done(u) | UnitSlot::InProgress(u)) => {
                self.units[*u].kind == UnitKind::Entity
            }
            Some(UnitSlot::NotLoaded { file, .. }) => {
                match self.files.get(&(lib, file.clone())) {
                    // Not parsed (the analysed-unit cache registered it): the manifest knows.
                    Some(f) if !f.parsed => match &self.libs[lib].source {
                        LibSource::Manifest(m) => m.find(LibKind::Entity, key.as_str()).is_some(),
                        _ => false,
                    },
                    f => f.and_then(|f| f.units.as_ref()).is_some_and(|us| {
                        us.iter().any(|(k, u)| {
                            *k == pk
                                && matches!(u, Some(du) if matches!(du.unit, ast::LibraryUnit::Entity(_)))
                        })
                    }),
                }
            }
            Some(UnitSlot::Failed) => false,
            None => match &self.libs[lib].source {
                LibSource::Manifest(m) => m.find(LibKind::Entity, key.as_str()).is_some(),
                _ => false,
            },
        }
    }

    /// The library of the entity default binding picks for component `comp` instantiated
    /// in `scope` (7.3.3, Q4): an entity made directly visible by a use clause, else the
    /// one in the library of the design unit that declares the component (#291), else the
    /// one in the library of the instantiating design unit. The second value is the `work`
    /// candidate when it differs, and the third says what the first one is.
    fn default_entity_lib(
        &self,
        scope: ScopeId,
        comp: DeclId,
    ) -> (Option<LibId>, Option<LibId>, &'static str) {
        let key = self.decls[comp].name.key();
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
        let has = |l: Option<LibId>| l.filter(|&l| self.has_entity(l, key));
        let work = has(self.unit_lib_of(scope));
        let declared = has(self.unit_lib_of(self.decls[comp].scope));
        let (first, why) = match (visible, declared) {
            (Some(v), _) => (Some(v), "directly visible through a use clause"),
            (None, Some(c)) => (Some(c), "in the library of the component declaration"),
            (None, None) => (work, "in the library of this design unit"),
        };
        (first, work.filter(|&w| Some(w) != first), why)
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
    fn note_default_binding(&mut self, scope: ScopeId, comp: DeclId, span: Span) {
        let key = self.decls[comp].name.key();
        let (chosen, other, why) = self.default_entity_lib(scope, comp);
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
                    "`{a}.{key}` is {why}; `{b}` is the library of this design unit (LRM 7.3.3)"
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
        let lib = self.default_entity_lib(scope, comp).0?;
        let entity = self.load_unit(lib, UnitKey::Primary(key), None).ok()?;
        let arch = self.last_architecture(entity);
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
        // The hidden types of anonymous interface types come last and take no actual.
        let visible = formals.iter().filter(|&&f| !self.hidden_type(f)).count();
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
                if k >= visible {
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
            if k >= visible {
                continue;
            }
            let m_idx = match &m {
                Assoc::Whole { idx, .. } => Some(*idx),
                _ => None,
            };
            let actual = match m {
                // After an association error a missing actual is likely a cascade.
                Assoc::None if errs.0 => None,
                Assoc::None => errs.keep(self.missing_actual(scope, f, at)),
                Assoc::Whole { idx, conv, span } => {
                    errs.keep(self.actual(scope, f, &assocs[idx].actual, conv, span))
                }
                Assoc::Parts(parts) => errs.keep(self.partial(scope, f, assocs, &parts)),
            };
            // Later generic subprograms and the port map see this type's actual.
            if let (Some(Actual::Subtype(st)), DeclKind::GenericType(t)) =
                (&actual, &self.decls[f].kind)
            {
                let (t, st) = (*t, *st);
                self.inst_types.push((t, st));
                // Its anonymous index and element types too (2019 6.5.3).
                let parts = self.anonymous_parts(t, st);
                self.inst_types.extend(parts);
            }
            if let Some(actual) = actual {
                // The actual of an `out` or `linkage` formal is written, not read.
                if matches!(
                    &self.decls[f].kind,
                    DeclKind::Object(o) if matches!(o.mode, Some(Mode::Out | Mode::Linkage))
                ) {
                    self.not_read_actual(&actual);
                }
                args.push(Arg {
                    formal: k as u16,
                    actual,
                    inertial: matches!(m_idx, Some(i) if matches!(assocs[i].actual, ast::Actual::Inertial(_))),
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
                format!(
                    "too many actuals: {owner} has {} {what}s",
                    formals.iter().filter(|&&f| !self.hidden_type(f)).count()
                ),
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
        // 2019 `g [t return t] => actual` names one formal subprogram (LCS-2016-I03).
        let f = match &f.kind {
            ast::NameKind::Signature { prefix, signature } => {
                if let ast::NameKind::Simple(i) = &prefix.kind
                    && let Some(&p) = self
                        .formals_with_signature(scope, formals, i.key, signature)?
                        .first()
                {
                    return Ok((p, None, true));
                }
                prefix.as_ref()
            }
            _ => f,
        };
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
        let d = Diagnostic::error(
            diag::NOT_DECLARED,
            format!("{owner} has no {what} `{name}`"),
        )
        .with_primary(span, format!("not a {what} of {owner}"));
        let found = root.map_or_else(Vec::new, |r| {
            crate::suggest::similar(
                r.key.as_str(),
                formals.iter().filter_map(|&f| self.candidate(f)),
            )
        });
        let d = self.suggest_decls(d, span, &found);
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
            crate::scope::Visible::Overloads(ds) => ds.to_vec(),
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

    /// Whether `d` is the hidden generic type of an anonymous interface type (`c : type
    /// is (<>)`), which an entity lists after its generics (#283).
    pub(crate) fn hidden_type(&self, d: DeclId) -> bool {
        matches!(self.decls[d].kind, DeclKind::GenericType(t) if self.types[t].anonymous)
    }

    /// `E1311`/`E1310`: the anonymous type of `f` comes from its actual, so `open`, an
    /// absent association and a default cannot bind it.
    pub(crate) fn unbound_anonymous(&mut self, f: DeclId, at: Span) -> Reported {
        let name = self.decls[f].name;
        self.error(
            Diagnostic::error(
                diag::MISSING_GENERIC,
                format!("`{name}` has an anonymous type and needs an actual to bind it"),
            )
            .with_primary(at, "no actual here")
            .with_secondary(self.decls[f].span, "declared with `type is ...`")
            .with_note("LRM 6.5.3: the type of an object declared with an unspecified type indication is the type of its actual"),
        )
    }

    /// The actual of an object of anonymous type `t`: typed without context, a universal
    /// literal as INTEGER or REAL (L93); its class is checked against the type's (`E1503`).
    pub(crate) fn anonymous_actual(
        &mut self,
        scope: ScopeId,
        e: &ast::Expr,
        t: TypeId,
    ) -> Result<ExprId, Reported> {
        let mut id = self.expr(scope, e, crate::expr::Expected::Any);
        let ut = self.base(self.exprs[id].subtype);
        let wk = &self.well_known;
        let own = if Some(ut) == wk.universal_integer {
            wk.integer
        } else if Some(ut) == wk.universal_real {
            wk.real
        } else {
            None
        };
        if let Some(own) = own {
            id = self.expr(scope, e, crate::expr::Expected::Type(own));
        }
        if self.is_poison_expr(id) {
            return Err(Reported);
        }
        let st = self.exprs[id].subtype;
        self.check_class(t, st, e.span, &mut crate::generic::Remap::default())?;
        Ok(id)
    }

    /// Add the actual of each hidden type of `gens` to the generic map: the subtype of
    /// the actual of the generic or port that has the anonymous type (#283).
    pub(crate) fn with_hidden_args(
        &mut self,
        gens: &[DeclId],
        ports: &[DeclId],
        gmap: Box<[Arg]>,
        pmap: &[Arg],
        at: Span,
    ) -> Result<Box<[Arg]>, Reported> {
        let g0 = gmap.clone();
        let mut out = gmap.into_vec();
        let mut errs = Errs::default();
        for (k, &h) in gens.iter().enumerate() {
            let DeclKind::GenericType(t) = self.decls[h].kind else {
                continue;
            };
            if !self.hidden_type(h) {
                continue;
            }
            let has = |o: &DeclId| matches!(&self.decls[*o].kind, DeclKind::Object(x) if self.base(x.subtype) == t);
            let (args, i, o) = if let Some(i) = gens.iter().position(has) {
                (&g0[..], i, gens[i])
            } else if let Some(i) = ports.iter().position(has) {
                (pmap, i, ports[i])
            } else {
                return Err(self.ice(at, "an anonymous type without its object"));
            };
            let st = args
                .iter()
                .find(|a| a.formal as usize == i)
                .and_then(|a| match a.actual {
                    Actual::Expr(x) => Some(self.exprs[x].subtype),
                    _ => None,
                });
            match st {
                Some(st) => out.push(Arg {
                    formal: k as u16,
                    actual: Actual::Subtype(st),
                    inertial: false,
                }),
                None => {
                    errs.keep::<()>(Err(self.unbound_anonymous(o, at)));
                }
            }
        }
        errs.finish(out.into())
    }

    fn missing_actual(&mut self, scope: ScopeId, f: DeclId, at: Span) -> Result<Actual, Reported> {
        let o = match &self.decls[f].kind {
            DeclKind::Object(o) => o,
            DeclKind::GenericSubprogram(_) => return self.subprogram_default(scope, f, at),
            DeclKind::Error => return Err(Reported),
            _ => {
                let name = self.decls[f].name;
                return Err(self.error(
                    Diagnostic::error(
                        diag::MISSING_GENERIC,
                        format!("generic `{name}` has no actual"),
                    )
                    .with_primary(at, format!("`{name}` is not associated"))
                    .with_secondary(self.decls[f].span, "declared here")
                    .with_help(format!("add `generic map ({name} => ...)`")),
                ));
            }
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
            (ObjRole::Port, _) if o.class == ObjClass::Variable => {
                Err(self.variable_port_open(f, at, "is not associated"))
            }
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

    /// A 2019 variable port denotes its actual's object, so it must have one (6.5.6.3).
    fn variable_port_open(&mut self, f: DeclId, at: Span, label: &str) -> Reported {
        let name = self.decls[f].name;
        self.error(
            Diagnostic::error(
                diag::MISSING_ACTUAL,
                format!("variable port `{name}` has no actual"),
            )
            .with_primary(at, label.to_string())
            .with_secondary(self.decls[f].span, "declared here")
            .with_help(format!(
                "associate it with a shared variable or a variable port (`{name} => ...`)"
            )),
        )
    }

    /// A range or subtype indication as the actual of an object formal (only a generic type
    /// takes a subtype indication, 6.5.7.2).
    fn not_an_expression_actual(&mut self, a: &ast::Actual, at: Span) -> Reported {
        let what = if matches!(a, ast::Actual::Subtype(_)) {
            "a subtype indication"
        } else {
            "a range"
        };
        let span = crate::generic::actual_span(a);
        self.error(
            Diagnostic::error(
                diag::NOT_AN_OBJECT,
                format!("{what} is not an expression and cannot be the actual of an object"),
            )
            .with_primary(span, "not an expression")
            .with_secondary(at, "associated with this formal"),
        )
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
        match &self.decls[f].kind {
            DeclKind::GenericType(_) => return self.type_actual(scope, a, fspan),
            DeclKind::GenericSubprogram(_) => return self.subprogram_actual(scope, f, a, fspan),
            DeclKind::GenericPackage(_) => {
                return Ok(Actual::Package(
                    self.entity_package_actual(scope, f, a, fspan)?,
                ));
            }
            _ => {}
        }
        let DeclKind::Object(o) = &self.decls[f].kind else {
            return Err(self.ice(fspan, "formal generic of an unknown class"));
        };
        let (st, role, mode, has_default, class) = (
            o.subtype,
            o.role.clone(),
            o.mode,
            o.default.is_some(),
            o.class,
        );
        // A port of a generic type of this instance has the type's actual.
        // ponytail: only ports whose type is the generic type itself; composites built from
        // it need the instance substitution of generic.rs.
        let mut st = self
            .inst_types
            .iter()
            .find(|(t, _)| *t == self.base(st))
            .map_or(st, |&(_, a)| a);
        // An anonymous interface type (`type is ...`) is the type of the actual (#283).
        let anon = self.anonymous_generic(st).then(|| self.base(st));
        let e = match a {
            ast::Actual::Open(sp) if anon.is_some() => {
                return Err(self.unbound_anonymous(f, *sp));
            }
            ast::Actual::Open(sp) if class == ObjClass::Variable && role == ObjRole::Port => {
                return Err(self.variable_port_open(f, *sp, "left open here"));
            }
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
            ast::Actual::Subtype(_) | ast::Actual::Range(_) => {
                return Err(self.not_an_expression_actual(a, fspan));
            }
        };
        // Decided on the rule 1 review (F8, #111): no bounds flow back through a formal
        // conversion. A formal that is not fully constrained takes them from the conversion
        // instead (5.3.2.2): the parameter subtype of a function, a type mark's subtype.
        // ponytail: the conversion must be fully constrained, not just where the formal is
        // open (5.3.2.2 asks only for the missing index ranges).
        let full = crate::types::Constrainedness::Full;
        let formal_open = self.constrainedness(st) != full;
        let into = formal_conv.and_then(|c| match &self.decls[c].kind {
            DeclKind::Type(t) => Some(self.types[*t].first_subtype),
            DeclKind::Subtype(s) => Some(*s),
            k => k
                .subprogram()
                .and_then(|s| s.params.first())
                .map(|p| p.subtype),
        });
        if formal_open && into.is_some_and(|c| self.constrainedness(c) != full) {
            let name = self.decls[f].name;
            return Err(self.error(
                Diagnostic::error(
                    diag::FORMAL_CONV_UNCONSTRAINED,
                    format!("a conversion of formal `{name}` needs a fully constrained formal or conversion"),
                )
                .with_primary(fspan, format!("`{name}` is of the unconstrained subtype `{}`", self.subtype_name(st)))
                .with_help("use a constrained subtype as the type mark or as the function's parameter, or constrain the port"),
            ));
        }
        // With a formal conversion the actual has the conversion's result type, unless the
        // actual is itself a conversion (`conv(f) => conv(a)`, inout): that one maps the
        // actual to the formal's type (6.5.7.1).
        let actual_converts = formal_conv.is_some()
            && mode != Some(Mode::Out)
            && mode != Some(Mode::Buffer)
            && matches!(&e.kind, ast::ExprKind::Name(n)
                if matches!(n.kind, ast::NameKind::Apply { .. }))
            && {
                let c = self.cands(scope, e);
                self.best_cost(&c, crate::expr::Expected::Type(self.base(st)))
                    .is_some()
            };
        let back = match formal_conv {
            Some(c) => match &self.decls[c].kind {
                DeclKind::Type(t) => self.types[*t].first_subtype,
                DeclKind::Subtype(s) => *s,
                k => k.subprogram().and_then(|s| s.ret).unwrap_or(st),
            },
            None => st,
        };
        // A formal conversion that is a type mark converts the formal to the actual's type
        // (9.3.6): the two must be closely related.
        if let Some(c) = formal_conv
            && matches!(self.decls[c].kind, DeclKind::Type(_) | DeclKind::Subtype(_))
            && !self.closely_related(self.base(st), self.base(back))
        {
            let (from, to) = (
                self.type_name(self.base(st)),
                self.type_name(self.base(back)),
            );
            return Err(self.error(
                Diagnostic::error(
                    diag::BAD_CONVERSION,
                    format!("cannot convert `{from}` to `{to}`"),
                )
                .with_primary(fspan, "these types are not closely related")
                .with_note("9.3.6: a type mark as formal part is a type conversion of the formal"),
            ));
        }
        let id = match anon {
            Some(_) if formal_conv.is_some() => {
                let name = self.decls[f].name;
                return Err(self.error(
                    Diagnostic::error(
                        diag::ANONYMOUS_FORMAL_CONVERSION,
                        format!("the formal `{name}` has an anonymous type and cannot be converted"),
                    )
                    .with_primary(fspan, "conversion of a formal of an anonymous type")
                    .with_secondary(self.decls[f].span, "declared with `type is ...`")
                    .with_note("LRM 6.5.3: the type of such an object is the type of its actual, which a conversion on the formal side would have to supply")
                    .with_help("associate the formal directly and convert in the design"),
                ));
            }
            Some(t) => {
                let id = self.anonymous_actual(scope, e, t)?;
                st = self.exprs[id].subtype;
                // Later objects of the same type (`k : c'subtype`) take it.
                self.inst_types.push((t, st));
                id
            }
            None => self.expr_in_subtype(scope, e, if actual_converts { st } else { back }),
        };
        if self.is_poison_expr(id) {
            return Err(Reported);
        }
        if role != ObjRole::Port {
            return Ok(Actual::Expr(id));
        }
        // A 2019 variable port takes a shared variable or another variable port (6.5.7.1).
        if class == ObjClass::Variable {
            let ok = formal_conv.is_none()
                && matches!(a, ast::Actual::Expr(_))
                && matches!(self.exprs[id].kind, ExprKind::Obj(d)
                if matches!(&self.decls[d].kind, DeclKind::Object(o)
                    if o.class == ObjClass::SharedVariable || (o.class == ObjClass::Variable && o.role == ObjRole::Port)));
            if !ok {
                return Err(self.error(
                    Diagnostic::error(
                        diag::CLASS_MISUSE,
                        "the actual of a variable port must be a shared variable or a variable port",
                    )
                    .with_primary(self.exprs[id].span, "not a shared variable"),
                ));
            }
            return Ok(Actual::Expr(id));
        }
        let actual_conv = self.port_conversion(scope, a, id);
        // An actual conversion feeding a formal that is not fully constrained gives it its
        // bounds (5.3.2.2): the function's result subtype or the type mark must define them.
        if actual_conv.is_some()
            && formal_open
            && mode != Some(Mode::Out)
            && mode != Some(Mode::Buffer)
            && self.constrainedness(self.exprs[id].subtype) != full
        {
            let name = self.decls[f].name;
            return Err(self.error(
                Diagnostic::error(
                    diag::FORMAL_CONV_UNCONSTRAINED,
                    format!("the conversion of the actual of unconstrained formal `{name}` must give its bounds"),
                )
                .with_primary(self.exprs[id].span, format!("of the unconstrained subtype `{}`", self.subtype_name(self.exprs[id].subtype)))
                .with_help("use a constrained subtype as the type mark, or a function with a constrained result subtype"),
            ));
        }
        // An `inout` port converted on the actual side only would send the formal's value
        // back unconverted (2 of 3 reference simulators reject it, L72). A formal conversion
        // alone is legal: copy-out converts, copy-in passes the actual unchanged, so the
        // actual's type must also be the formal's.
        if mode == Some(Mode::Inout) && actual_conv.is_some() && formal_conv.is_none() {
            let name = self.decls[f].name;
            return Err(self.error(
                Diagnostic::error(
                    diag::CONV_MISUSE,
                    format!("`inout` port `{name}` is converted the way in, but not the way out"),
                )
                .with_primary(
                    self.exprs[id].span,
                    "the only conversion of this association",
                )
                .with_help(
                    "convert both ways, `conv(formal) => conv(actual)`, or convert the formal only",
                ),
            ));
        }
        if mode == Some(Mode::Inout) && formal_conv.is_some() && actual_conv.is_none() {
            let (ft, at) = (self.base(st), self.base(self.exprs[id].subtype));
            if ft != at {
                let (name, ftn, atn) = (self.decls[f].name, self.type_name(ft), self.type_name(at));
                return Err(self.error(
                    Diagnostic::error(
                        diag::NO_OVERLOAD,
                        format!("`inout` port `{name}` of type `{ftn}` is converted the way out only, but the actual is of type `{atn}`"),
                    )
                    .with_primary(self.exprs[id].span, format!("expected type `{ftn}` to pass in unchanged"))
                    .with_help("convert the actual too, `conv(formal) => conv(actual)`"),
                ));
            }
        }
        if let Some((conv, x)) = actual_conv
            && mode != Some(Mode::Out)
            && mode != Some(Mode::Buffer)
        {
            // The way out: the formal part gives the actual's type.
            let want = self.base(self.exprs[x].subtype);
            if formal_conv.is_some() && self.base(back) != want {
                let (name, tn) = (self.decls[f].name, self.type_name(want));
                return Err(self.error(
                    Diagnostic::error(
                        diag::NO_OVERLOAD,
                        format!("no conversion of formal `{name}` gives the actual's type `{tn}`"),
                    )
                    .with_primary(fspan, format!("expected a conversion to `{tn}`"))
                    .with_help("use a function of one parameter of the formal's type, or a type conversion"),
                ));
            }
            return Ok(Actual::Converted {
                formal_conv,
                actual_conv: conv,
                actual: x,
                copy_in: None,
                copy_out: None,
            });
        }
        if mode != Some(Mode::In) && matches!(a, ast::Actual::Inertial(_)) {
            return Err(self.error(
                Diagnostic::error(
                    diag::CLASS_MISUSE,
                    "an `inertial` actual (an expression) is only allowed for an `in` port",
                )
                .with_primary(self.exprs[id].span, "not allowed here"),
            ));
        }
        self.check_written_actual(f, mode, id)?;
        Ok(match formal_conv {
            Some(_) => Actual::Converted {
                formal_conv,
                actual_conv: None,
                actual: id,
                copy_in: None,
                copy_out: None,
            },
            None => Actual::Expr(id),
        })
    }

    /// The conversion of port actual `a` (resolved as `id`) written `conv(name)` of a signal
    /// name (6.5.7.1): the function or type mark and the name; `None` for an expression.
    fn port_conversion(
        &mut self,
        scope: ScopeId,
        a: &ast::Actual,
        id: ExprId,
    ) -> Option<(Option<DeclId>, ExprId)> {
        // Port actuals (6.5.6.3, 6.5.7.1): a conversion of a signal, or for
        // modes other than `in` a signal name. A conversion is written `name(actual)`: an
        // operator (`not s`) or an `inertial` actual is an expression.
        let call_syntax =
            matches!(a, ast::Actual::Expr(x) if matches!(x.kind, ast::ExprKind::Name(_)));
        // The type mark of a type conversion actual, by its own declaration: a subtype keeps
        // the constraint that bounds an unconstrained formal (5.3.2.2).
        let mark = match (&self.exprs[id].kind, a) {
            (
                ExprKind::Conv { .. },
                ast::Actual::Expr(ast::Expr {
                    kind: ast::ExprKind::Name(n),
                    ..
                }),
            ) => match &n.kind {
                ast::NameKind::Apply { prefix, .. } => Some(prefix),
                _ => None,
            },
            _ => None,
        };
        let mark = mark
            .and_then(|p| self.expanded_name_decl(scope, p).ok())
            .map(|d| self.entity(d))
            .filter(|&d| matches!(self.decls[d].kind, DeclKind::Subtype(_)));
        match &self.exprs[id].kind {
            _ if !call_syntax => None,
            // A predefined operator in call syntax (`"not"(s)`) is an expression, not a
            // conversion function: it has no body to convert with.
            ExprKind::Call { callee, args, .. }
                if args.len() == 1
                    && !self.decls[*callee].kind.subprogram().is_some_and(|sp| {
                        matches!(
                            sp.body,
                            crate::decl::Body::Implicit(_) | crate::decl::Body::Native(_)
                        )
                    }) =>
            {
                match &args[0].actual {
                    Actual::Expr(x) if self.is_signal_name(*x) => Some((Some(*callee), *x)),
                    _ => None,
                }
            }
            ExprKind::Conv { to, operand } if self.is_signal_name(*operand) => {
                let t = self.types[self.subtypes[*to].base].decl;
                Some((Some(mark.unwrap_or(t)), *operand))
            }
            _ => None,
        }
    }

    /// The actual `id` of port `f` (or of a part of it) with `mode`: a formal that is
    /// written needs a signal name that can be written (6.5.6.3, 6.5.7.1).
    fn check_written_actual(
        &mut self,
        f: DeclId,
        mode: Option<Mode>,
        id: ExprId,
    ) -> Result<(), Reported> {
        // 6.5.6.3: a formal that is written (out, inout, buffer) needs an
        // actual that can be written; an `in` port or parameter cannot (#654). A linkage
        // formal takes an actual of any mode.
        if matches!(mode, Some(Mode::Out | Mode::Inout | Mode::Buffer)) {
            let mut e = id;
            while let Some((_, operand)) = self.actual_conversion(e) {
                e = operand;
            }
            if let Some(ObjRoot::Decl(d)) = self.lsp(e).map(|l| l.root)
                && let DeclKind::Object(o) = &self.decls[d].kind
                && o.mode == Some(Mode::In)
                && matches!(o.role, ObjRole::Port | ObjRole::Param)
            {
                let (an, am) = (self.decls[d].name, mode_str(o.mode));
                let (fname, fm) = (self.decls[f].name, mode_str(mode));
                return Err(self.error(
                        Diagnostic::error(
                            diag::CLASS_MISUSE,
                            format!("{an} with mode {am} cannot be associated with formal {fname} with mode {fm}"),
                        )
                        .with_primary(self.exprs[id].span, format!("mode {am} cannot be written"))
                        .with_secondary(self.decls[f].span, "formal declared here")
                        .with_note("LRM 6.5.6.3: the actual of a formal that is written must itself be writable"),
                    ));
            }
        }
        // 1993 1.1.1.2: a `buffer` port is associated only with a `buffer` port (besides an
        // `in` or `linkage` formal); 2008 6.5.6.3 relaxed it (#814).
        if self.file_rev() == crate::LangRev::Vhdl1993
            && matches!(&self.decls[f].kind, DeclKind::Object(o) if o.role == ObjRole::Port)
            && let Some(ObjRoot::Decl(d)) = self.lsp(id).map(|l| l.root)
            && let DeclKind::Object(o) = &self.decls[d].kind
            && o.role == ObjRole::Port
            && let Some(am) = o.mode
            && matches!(
                (am, mode),
                (Mode::Buffer, Some(Mode::Out | Mode::Inout))
                    | (Mode::Out | Mode::Inout, Some(Mode::Buffer))
            )
        {
            let an = self.decls[d].name;
            let (fname, fm) = (self.decls[f].name, mode_str(mode));
            let am = mode_str(Some(am));
            return Err(self.error(
                Diagnostic::error(
                    diag::NOT_IN_REVISION,
                    format!("port {an} with mode {am} cannot be associated with formal {fname} with mode {fm} in VHDL-1993"),
                )
                .with_primary(self.exprs[id].span, format!("mode {am}"))
                .with_secondary(self.decls[f].span, "formal declared here")
                .with_note("LRM 1993 1.1.1.2: a formal port of mode buffer takes only a port of mode buffer, and a port of mode buffer is the actual only of a formal of mode in, buffer or linkage (VHDL-2008 allows out and inout too)"),
            ));
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
        Ok(())
    }

    /// `t => integer`, `t => std_logic_vector(7 downto 0)`: a generic type's actual.
    fn type_actual(
        &mut self,
        scope: ScopeId,
        a: &ast::Actual,
        at: Span,
    ) -> Result<Actual, Reported> {
        let st = match a {
            ast::Actual::Subtype(si) => self.subtype_indication(scope, si)?,
            ast::Actual::Expr(ast::Expr {
                kind: ast::ExprKind::Name(n),
                ..
            }) => match self.index_constrained_name(scope, n) {
                Some(st) => st?,
                None => self.type_mark(scope, n)?,
            },
            _ => {
                return Err(self.error(
                    Diagnostic::error(
                        diag::NOT_A_TYPE,
                        "the actual of a generic type must be a subtype",
                    )
                    .with_primary(at, "not a subtype indication"),
                ));
            }
        };
        Ok(Actual::Subtype(st))
    }

    /// The default of generic subprogram `f` without an actual (6.5.6.2): `is <>` is the
    /// subprogram of the formal's designator visible at the instance, `is name` the one
    /// `name` denoted at the formal's declaration (L11), each with a conforming profile.
    fn subprogram_default(
        &mut self,
        scope: ScopeId,
        f: DeclId,
        at: Span,
    ) -> Result<Actual, Reported> {
        let name = self.decls[f].name;
        let cands = match self.generic.defaults.get(&f).cloned() {
            Some(crate::generic::SubpDefault::Box) => match self.lookup(scope, name.key()) {
                Visible::One(d) => vec![d],
                Visible::Overloads(ds) => ds.to_vec(),
                _ => Vec::new(),
            },
            Some(crate::generic::SubpDefault::Name { cands, .. }) => cands,
            None => {
                return Err(self.error(
                    Diagnostic::error(
                        diag::MISSING_GENERIC,
                        format!("generic `{name}` has no actual and no default"),
                    )
                    .with_primary(at, format!("`{name}` is not associated"))
                    .with_secondary(self.decls[f].span, "declared here without a default")
                    .with_help(format!("add `generic map ({name} => ...)`")),
                ));
            }
        };
        self.fit_subprogram(f, cands, at, None)
    }

    /// `img => ii`: a subprogram whose profile is the formal's with the instance's type
    /// actuals substituted (6.5.7.2).
    fn subprogram_actual(
        &mut self,
        scope: ScopeId,
        f: DeclId,
        a: &ast::Actual,
        at: Span,
    ) -> Result<Actual, Reported> {
        let ast::Actual::Expr(ast::Expr {
            kind: ast::ExprKind::Name(n),
            ..
        }) = a
        else {
            return Err(self.error(
                Diagnostic::error(
                    diag::TYPE_MISMATCH,
                    "the actual of a generic subprogram must be a subprogram name",
                )
                .with_primary(at, "not a subprogram name"),
            ));
        };
        let cands = match self.classify(scope, n) {
            crate::expr::names::NameRes::One(d) => vec![d],
            crate::expr::names::NameRes::Overloads(ds) => ds,
            other => return Err(self.report_name(scope, other, n.span, "a function name")),
        };
        // The generic subprogram of a component or a block is not an actual (user, 2026-10-19, L63).
        if let Some(&d) = cands.iter().find(|&&d| {
            matches!(self.decls[d].kind, DeclKind::GenericSubprogram(_))
                && matches!(
                    self.scopes[self.decls[d].scope].kind,
                    ScopeKind::Component | ScopeKind::Block
                )
        }) {
            return Err(self.error(
                Diagnostic::error(
                    diag::GENERIC_SUBPROGRAM_ACTUAL,
                    format!(
                        "the generic subprogram `{}` of a component or block cannot be the actual of a generic subprogram",
                        self.decls[d].name
                    ),
                )
                .with_primary(n.span, "")
                .with_secondary(self.decls[d].span, "declared here")
                .with_help("name a subprogram declared in the architecture or a package; a default binding passes a component's generic subprogram by name"),
            ));
        }
        let text = self.source_text(n.span);
        self.fit_subprogram(f, cands, n.span, Some(&text))
    }

    /// The one candidate whose profile is generic subprogram `f`'s, with the instance's
    /// type actuals substituted; `actual`: the actual's text, `None` for the default.
    fn fit_subprogram(
        &mut self,
        f: DeclId,
        cands: Vec<DeclId>,
        span: Span,
        actual: Option<&str>,
    ) -> Result<Actual, Reported> {
        let subst = |db: &Db, st: SubtypeId| {
            let t = db.base(st);
            db.inst_types
                .iter()
                .find(|(g, _)| *g == t)
                .map_or(t, |&(_, a)| db.base(a))
        };
        let Some(fs) = self.decls[f].kind.subprogram() else {
            return Err(Reported);
        };
        let want = crate::scope::Profile {
            params: fs.params.iter().map(|p| subst(self, p.subtype)).collect(),
            ret: fs.ret.map(|r| subst(self, r)),
        };
        let fits: Vec<DeclId> = cands
            .into_iter()
            .map(|d| self.entity(d))
            .filter(|&d| self.profile_fits(f, &want, &self.profile(d)))
            .collect();
        match fits.as_slice() {
            [d] => Ok(Actual::Subprogram(*d)),
            _ => Err(self.error(
                match actual {
                    Some(text) => Diagnostic::error(
                        diag::TYPE_MISMATCH,
                        format!(
                            "no single visible `{text}` matches generic subprogram `{}`",
                            self.decls[f].name
                        ),
                    ),
                    None => Diagnostic::error(
                        diag::GENERIC_DEFAULT,
                        format!(
                            "no single visible subprogram for the default of generic subprogram `{}`",
                            self.decls[f].name
                        ),
                    ),
                }
                .with_primary(
                    span,
                    if fits.is_empty() {
                        "no subprogram with this profile"
                    } else {
                        "several subprograms fit"
                    },
                )
                .with_secondary(self.decls[f].span, "generic subprogram declared here"),
            )),
        }
    }

    fn is_signal_name(&self, e: ExprId) -> bool {
        match self.lsp(e).map(|l| l.root) {
            Some(r @ ObjRoot::Decl(_)) | Some(r @ ObjRoot::External(_)) => self.is_signal(r),
            Some(ObjRoot::ImplicitSignal(_)) => true,
            None => false,
        }
    }

    /// Sub-element associations of one formal (6.5.7.1): `f(0) => a, f(1) => b`.
    pub(crate) fn partial(
        &mut self,
        scope: ScopeId,
        f: DeclId,
        assocs: &[ast::AssocElem],
        parts: &[(usize, Span)],
    ) -> Result<Actual, Reported> {
        // The formal designator names `f`, but the index and range expressions
        // in it are evaluated where the association is (6.5.7.1): look the name
        // up in a region that holds only `f`, inside the instantiating one.
        let fscope = self.add_scope(crate::scope::ScopeKind::Block, Some(scope), None);
        let key = self.decls[f].name.key();
        self.scopes[fscope].names.insert(key, vec![f]);
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
            // The formal designator names the formal, it reads nothing.
            self.not_read(fe);
            let Some((root, steps, stat, cut)) = self.lsp_parts(fe) else {
                errs.keep::<()>(Err(self.ice(span, "formal sub-element without a prefix")));
                continue;
            };
            // `cut`: a non-static index or range, which `lsp_parts` drops from `steps`.
            if root != ObjRoot::Decl(f) || cut || stat < crate::stat::Staticness::Locally {
                errs.keep::<()>(Err(self.error(
                    Diagnostic::error(
                        diag::NOT_STATIC,
                        "a formal sub-element must be a locally static name",
                    )
                    .with_primary(span, "not locally static"),
                )));
                continue;
            }
            // 2019 (LCS-2016-001): an open sub-element is left unassociated, which a
            // port of mode `in` allows only with a default, as for a whole port (6.5.7.1).
            if let ast::Actual::Open(sp) = &a.actual {
                let in_no_default = matches!(&self.decls[f].kind, DeclKind::Object(o)
                    if o.mode == Some(Mode::In) && o.default.is_none());
                let msg = if self.file_rev() < crate::LangRev::Vhdl2019 {
                    "a sub-element of a formal cannot be `open` before VHDL-2019"
                } else if in_no_default {
                    "a sub-element of a port of mode `in` without a default cannot be `open`"
                } else {
                    continue;
                };
                errs.keep::<()>(Err(self.error(
                    Diagnostic::error(diag::MISSING_ACTUAL, msg)
                        .with_primary(*sp, "left open here"),
                )));
                continue;
            }
            let (ast::Actual::Expr(e) | ast::Actual::Inertial(e)) = &a.actual else {
                errs.keep::<()>(Err(self.not_an_expression_actual(&a.actual, span)));
                continue;
            };
            let inertial = matches!(a.actual, ast::Actual::Inertial(_));
            if inertial
                && matches!(&self.decls[f].kind, DeclKind::Object(o) if o.mode != Some(Mode::In))
            {
                errs.keep::<()>(Err(self.error(
                    Diagnostic::error(
                        diag::CLASS_MISUSE,
                        "an `inertial` actual (an expression) is only allowed for an `in` port",
                    )
                    .with_primary(e.span, "not allowed here"),
                )));
                continue;
            }
            let st = self.exprs[fe].subtype;
            let id = self.expr_in_subtype(scope, e, st);
            if self.is_poison_expr(id) {
                errs.0 = true;
                continue;
            }
            let port = match &self.decls[f].kind {
                DeclKind::Object(o) if o.role == ObjRole::Port && o.class == ObjClass::Signal => {
                    Some(o.mode)
                }
                _ => None,
            };
            // A part of a written signal port needs a writable signal name, as a whole port.
            if let Some(mode) = port
                && errs.keep(self.check_written_actual(f, mode, id)).is_none()
            {
                continue;
            }
            // `f.x => conv(s)` of an `in` port converts in the port association (#611).
            let conv = port == Some(Some(Mode::In))
                && self.port_conversion(scope, &a.actual, id).is_some();
            out.push((steps.into_boxed_slice(), id, fe, inertial, conv));
        }
        if !errs.0 && out.len() == parts.len() {
            let spans: Vec<Span> = parts.iter().map(|p| p.1).collect();
            if let Err(e) = self.check_contiguous(f, &out, &spans) {
                errs.keep::<()>(Err(e));
            }
        }
        errs.finish(Actual::Individual(out.into()))
    }

    /// 6.5.7.1: the sub-elements associated individually with an unconstrained
    /// one-dimensional array formal form a contiguous index range, each element once.
    /// Only locally static integer or enumeration indices and ranges are checked.
    // ponytail: scalar-index checks only; multidimensional formals and non-folded
    // bounds are not checked.
    fn check_contiguous(
        &mut self,
        f: DeclId,
        out: &[crate::hir::IndividualPart],
        spans: &[Span],
    ) -> Result<(), Reported> {
        use crate::hir::{Direction, LspStep, RangeC};
        let DeclKind::Object(o) = &self.decls[f].kind else {
            return Ok(());
        };
        let unconstrained = matches!(
            self.subtypes[o.subtype].constraint,
            crate::types::Constraint::None | crate::types::Constraint::Array { dims: None, .. }
        ) && matches!(
            self.types[self.base(o.subtype)].kind,
            crate::types::TypeKind::Array { .. }
        );
        if !unconstrained {
            return Ok(());
        }
        // (lo, hi, whole element, span) per association, null ranges dropped.
        let mut iv: Vec<(i64, i64, bool, Span)> = Vec::new();
        for ((steps, ..), &sp) in out.iter().zip(spans) {
            let (lo, hi) = match steps.first() {
                Some(LspStep::Index(ix)) if ix.len() == 1 => {
                    let Some(v) = self.value(ix[0]).and_then(|c| c.as_int()) else {
                        return Ok(());
                    };
                    (v, v)
                }
                Some(LspStep::Slice(RangeC::Explicit { left, dir, right })) => {
                    let (Some(l), Some(r)) = (
                        self.value(*left).and_then(|c| c.as_int()),
                        self.value(*right).and_then(|c| c.as_int()),
                    ) else {
                        return Ok(());
                    };
                    if *dir == Direction::To {
                        (l, r)
                    } else {
                        (r, l)
                    }
                }
                _ => return Ok(()),
            };
            if lo <= hi {
                iv.push((lo, hi, steps.len() == 1, sp));
            }
        }
        iv.sort_by_key(|&(lo, hi, ..)| (lo, hi));
        let mut next: Option<i64> = None;
        for &(lo, hi, whole, sp) in &iv {
            if let Some(n) = next {
                if lo > n {
                    let what = if lo - 1 == n {
                        format!("index {n} of the formal has no actual")
                    } else {
                        format!("indices {n} to {} of the formal have no actual", lo - 1)
                    };
                    return Err(self.error(
                        Diagnostic::error(
                            diag::MISSING_ACTUAL,
                            "gap in the individually associated formal",
                        )
                        .with_primary(sp, "elements of the formal are skipped before this one")
                        .with_note(what),
                    ));
                }
                if lo < n && whole {
                    return Err(self.error(
                        Diagnostic::error(
                            diag::FORMAL_TWICE,
                            "element of the formal is associated twice",
                        )
                        .with_primary(sp, "overlaps an earlier association")
                        .with_note(format!(
                            "index {lo} of the formal is associated more than once"
                        )),
                    ));
                }
            }
            next = Some(next.map_or(hi + 1, |n| n.max(hi + 1)));
        }
        Ok(())
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
            self.config_specs.edit(p.scope, |v| {
                v.push(ComponentConfig {
                    instances,
                    component: p.component,
                    binding: Some(p.binding),
                    block: None,
                    span: p.span,
                })
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
        let (comp_scope, locals_g, locals_p) = (c.scope, c.generics.clone(), c.ports.clone());
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
        // Generic types of the component and the entity that correspond by name (#222).
        let types: Vec<(TypeId, TypeId)> = locals_g
            .iter()
            .filter_map(|&l| match self.decls[l].kind {
                DeclKind::GenericType(a) => eg.iter().find_map(|&f| match self.decls[f].kind {
                    DeclKind::GenericType(b)
                        if self.decls[f].name.key() == self.decls[l].name.key() =>
                    {
                        Some((a, b))
                    }
                    _ => None,
                }),
                _ => None,
            })
            .collect();
        // The actuals see what is visible at the binding, with the component's own generics
        // and ports in front (7.3.2.2, #468).
        let cscope = self.add_scope(ScopeKind::Configuration, Some(scope), None);
        let (names, uses) = (
            self.scopes[comp_scope].names.clone(),
            self.scopes[comp_scope].uses.clone(),
        );
        self.scopes[cscope].names = names;
        self.scopes[cscope].uses = uses;
        // The generic map records the actuals of generic types in `inst_types` for
        // the port map of this binding only: two bindings of one generic entity get
        // one instance each, not the first binding's types (#614).
        self.inst_types.clear();
        let generic_map = match &b.generic_map {
            Some(m) => errs.keep(self.assoc_list(cscope, &eg, m, b.span)),
            None => errs.keep(self.default_map(&locals_g, &eg, e, &types, at)),
        };
        let port_map = match &b.port_map {
            Some(m) => errs.keep(self.assoc_list(cscope, &ep, m, b.span)),
            None => errs.keep(self.default_map(&locals_p, &ep, e, &types, at)),
        };
        self.inst_types.clear();
        // A binding indication connects the component's ports to the entity's, it reads no
        // value (LRM 1993 4.3.2): an `out` port of the component is no read there.
        for a in generic_map.iter().chain(&port_map).flat_map(|m| m.iter()) {
            self.not_read_actual(&a.actual);
        }
        let generic_map = match (generic_map, &port_map) {
            (Some(g), Some(p)) => errs.keep(self.with_hidden_args(&eg, &ep, g, p, b.span)),
            _ => None,
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
    /// formal, or of another type, is `E1703`. `types`: the component's generic types
    /// with the entity's of the same name, which stand for the same actual.
    fn default_map(
        &mut self,
        locals: &[DeclId],
        formals: &[DeclId],
        entity: UnitId,
        types: &[(TypeId, TypeId)],
        at: Span,
    ) -> Result<Box<[Arg]>, Reported> {
        let mut errs = Errs::default();
        for &l in locals {
            let key = self.decls[l].name.key();
            let hit = formals.iter().find(|&&f| self.decls[f].name.key() == key);
            let ok = hit.is_some_and(|&f| self.binds_by_name(l, f, types));
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
                Some(&l) => match self.decls[l].kind {
                    DeclKind::GenericType(t) => Actual::Subtype(self.types[t].first_subtype),
                    DeclKind::GenericSubprogram(_) => Actual::Subprogram(l),
                    DeclKind::GenericPackage(i) => Actual::Package(i),
                    _ => Actual::Expr(self.object_ref(l, at)),
                },
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
                inertial: false,
            });
        }
        Ok(args.into())
    }

    /// Whether component local `l` matches entity formal `f` of the same name in a
    /// default binding: objects of the same base type, generic types, subprograms of the
    /// same profile and formal packages of the same generic package (`types` as for
    /// `default_map`).
    fn binds_by_name(&self, l: DeclId, f: DeclId, types: &[(TypeId, TypeId)]) -> bool {
        let same = |a: SubtypeId, b: SubtypeId| {
            let (a, b) = (self.base(a), self.base(b));
            a == b || types.contains(&(a, b))
        };
        match (&self.decls[l].kind, &self.decls[f].kind) {
            (DeclKind::Object(a), DeclKind::Object(b)) => same(a.subtype, b.subtype),
            (DeclKind::GenericType(_), DeclKind::GenericType(_)) => true,
            (DeclKind::GenericSubprogram(a), DeclKind::GenericSubprogram(b)) => {
                a.kind == b.kind
                    && a.params.len() == b.params.len()
                    && a.params
                        .iter()
                        .zip(&b.params)
                        .all(|(x, y)| same(x.subtype, y.subtype))
                    && match (a.ret, b.ret) {
                        (Some(x), Some(y)) => same(x, y),
                        (None, None) => true,
                        _ => false,
                    }
            }
            (DeclKind::GenericPackage(a), DeclKind::GenericPackage(b)) => {
                self.insts[*a].generic == self.insts[*b].generic
            }
            _ => false,
        }
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
        if !self.unit_known(lib, key) {
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
                    // Instances named by earlier component configurations (for `others`).
                    let named: Vec<StmtId> = out
                        .iter()
                        .filter_map(|it| match it {
                            ConfigItem::Component(ComponentConfig {
                                instances: InstanceSel::Labels(ls),
                                ..
                            }) => Some(ls.iter().copied()),
                            _ => None,
                        })
                        .flatten()
                        .collect();
                    let c = self.component_config(scope, regions, named_in, &named, c);
                    if let Some(c) = errs.keep(c) {
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
                let (s, param) = (*s, *param);
                let select = match arg {
                    None => None,
                    // `for g(r'range)` and `for g(subtype_name)`: a discrete range written as a
                    // name (1.3.1), not an index value.
                    Some(ast::Actual::Expr(e)) if matches!(&e.kind, ast::ExprKind::Name(n) if self.is_range_name(scope, n)) =>
                    {
                        let ast::ExprKind::Name(n) = &e.kind else {
                            unreachable!("checked above");
                        };
                        let st = self.formal_subtype(param).unwrap_or(self.poison.subtype);
                        let base = self.base(st);
                        Some(GenSelect::Range(self.range_name(scope, n, base)?))
                    }
                    Some(ast::Actual::Expr(e)) => {
                        let st = self.formal_subtype(param).unwrap_or(self.poison.subtype);
                        let id = self.expr_in_subtype(scope, e, st);
                        if self.is_poison_expr(id) {
                            return Err(Reported);
                        }
                        Some(GenSelect::Index(id))
                    }
                    Some(ast::Actual::Range(r)) => {
                        let st = self.formal_subtype(param).unwrap_or(self.poison.subtype);
                        let base = self.base(st);
                        Some(GenSelect::Range(self.range(scope, r, base)?))
                    }
                    Some(ast::Actual::Subtype(si)) => {
                        // `for g(natural range 1 to 3)`: the loop parameter's type, constrained.
                        let want = self.formal_subtype(param).map(|p| self.base(p));
                        let st = self.subtype_indication(scope, si)?;
                        let t = self.base(st);
                        if let Some(w) = want
                            && w != t
                            && w != self.poison.ty
                            && t != self.poison.ty
                        {
                            let n = self.type_names(&[w, t]);
                            return Err(self.error(
                                Diagnostic::error(
                                    diag::TYPE_MISMATCH,
                                    format!(
                                        "expected a range of `{}`, found one of `{}`",
                                        n[0], n[1]
                                    ),
                                )
                                .with_primary(si.span, "wrong type for this generate parameter"),
                            ));
                        }
                        Some(GenSelect::Range(crate::types::RangeC::Attr {
                            prefix: crate::hir::AttrPrefix::Subtype(st),
                            reverse: false,
                            dim: 1,
                        }))
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
        // The block configuration sees the declarations of its block, such as a component
        // declared there (3.4.2, #473).
        let first = inner[0];
        let names: Vec<(Symbol, Vec<DeclId>)> = self.scopes[first]
            .names
            .iter()
            .map(|(k, v)| (*k, v.clone()))
            .collect();
        for (k, ds) in names {
            self.scopes[bscope].names.entry(k).or_default().extend(ds);
        }
        let uses = self.scopes[first].uses.clone();
        self.scopes[bscope].uses.extend(uses);
        for u in &b.uses {
            self.use_clause(bscope, u);
        }
        let items = self.config_items(bscope, &inner, first, &b.items)?;
        Ok(BlockConfig {
            target,
            items,
            span: b.span,
        })
    }

    /// Whether `n` is a range attribute (`a'range`, `t'reverse_range(2)`) or a type mark.
    fn is_range_name(&mut self, scope: ScopeId, n: &ast::Name) -> bool {
        let attr = match &n.kind {
            ast::NameKind::Attribute { attr, .. } => Some(attr),
            ast::NameKind::Apply { prefix, .. } => match &prefix.kind {
                ast::NameKind::Attribute { attr, .. } => Some(attr),
                _ => None,
            },
            _ => None,
        };
        match attr {
            Some(a) => matches!(a.key.as_str(), "range" | "reverse_range"),
            None => self.quiet_type_mark(scope, n).is_some(),
        }
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
        named: &[StmtId],
        c: &ast::ComponentConfig,
    ) -> Result<ComponentConfig, Reported> {
        let comp = self.component_of_spec(scope, &c.spec)?;
        let instances =
            self.instance_sel(regions, &Labels::of(&c.spec.instances), comp, named_in)?;
        // An earlier configuration specification of the same instances (7.3.2.?):
        // incremental binding may add maps but not rebind.
        let spec = self.spec_for(regions, &instances, named, comp);
        let bound = spec.as_ref().and_then(|a| match a {
            Some(EntityAspect::Entity { unit, .. }) => Some(*unit),
            _ => None,
        });
        let binding = match &c.binding {
            Some(b) => {
                let bi = self.binding_indication(scope, comp, b, bound)?;
                // 7.3.2.1: an incremental binding may repeat the primary binding's entity
                // aspect; only a different one is an error.
                if let (Some(Some(old)), Some(new)) = (&spec, &bi.aspect) {
                    let same = match (old, new) {
                        (
                            EntityAspect::Entity { unit: u1, arch: a1 },
                            EntityAspect::Entity { unit: u2, arch: a2 },
                        ) => u1 == u2 && a1.map(|i| i.key) == a2.map(|i| i.key),
                        (EntityAspect::Configuration(u1), EntityAspect::Configuration(u2)) => {
                            u1 == u2
                        }
                        (EntityAspect::Open, EntityAspect::Open) => true,
                        _ => false,
                    };
                    if !same {
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
                            .with_primary(b.span, "bound to a different entity aspect here")
                            .with_help("an incremental binding may repeat the entity aspect of the primary binding or add generic and port maps: drop the entity aspect or make it the same"),
                        ));
                    }
                }
                Some(bi)
            }
            None => None,
        };
        let block = match &c.block {
            Some(bc) => {
                let e = match binding.as_ref().and_then(|b| b.aspect.as_ref()) {
                    Some(EntityAspect::Entity { unit, .. }) => Some(*unit),
                    _ => bound,
                };
                // Without an explicit binding the default binding applies (3.4.3, 7.3.3): the
                // LRM does not forbid configuring it (user, 2026-10-19, rule 15, decisions.md).
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
        named: &[StmtId],
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
                    // `others` leaves out the instances earlier component configurations name.
                    (InstanceSel::Labels(a), InstanceSel::Others) => {
                        a.iter().any(|x| !named.contains(x))
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
