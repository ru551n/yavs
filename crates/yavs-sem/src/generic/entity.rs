//! Design entities with generic types, subprograms or packages (2008 6.5.6): one clone of
//! the entity and architecture per distinct map of those actuals, with the formals replaced
//! by the actuals (5.1, 5.3). Generic constants stay generics of the clone: their values
//! are instance data that elab evaluates per instance. Components get the same treatment
//! (`instantiate_component`, per map, for elab), blocks are instantiated in place at the
//! end of their architecture (`instantiate_blocks`, one generic map each). #222.

use yavs_diag::Span;
use yavs_syntax::Symbol;

use super::{GenericMap, Remap};
use crate::db::{Db, Reported};
use crate::decl::{Component, Decl, DeclKind};
use crate::hir::{
    Actual, Arg, Binding, ComponentConfig, EntityAspect, InstanceSel, StmtKind, UnitHir,
};
use crate::ids::{DeclId, InstId, ScopeId, StmtId, SubtypeId, UnitId};
use crate::loader::Unit;

/// The actual of a generic type, subprogram or package of a design entity, as the HIR
/// generic map gives it (`Actual::Subtype`, `Actual::Subprogram`, `Actual::Package`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum GenericActual {
    Subtype(SubtypeId),
    Subprogram(DeclId),
    Package(InstId),
}

impl Db {
    /// Whether `entity` has a generic type, subprogram or package.
    pub fn has_non_constant_generics(&self, entity: UnitId) -> bool {
        match &self.units[entity].hir {
            UnitHir::Entity { generics, .. } => generics
                .iter()
                .any(|&g| !matches!(self.decls[g].kind, DeclKind::Object(_))),
            _ => false,
        }
    }

    /// The clone of `entity` and its architecture `arch` for these actuals (one per
    /// formal of the entity, `None` for generic constants): `(entity, architecture)`.
    /// The clone's generic list keeps only the generic constants, in order. Cloned once
    /// per distinct map; its identity is the entity's mangled name plus the actuals' (5.3).
    pub fn instantiate_entity(
        &mut self,
        entity: UnitId,
        arch: UnitId,
        actuals: &[Option<GenericActual>],
        at: Span,
    ) -> Result<(UnitId, UnitId), Reported> {
        let key = (entity, arch, actuals.to_vec().into_boxed_slice());
        if let Some(&c) = self.generic.entities.get(&key) {
            return Ok(c);
        }
        let UnitHir::Entity {
            generics,
            ports,
            stmts,
        } = &self.units[entity].hir
        else {
            return Err(self.ice(at, "instantiate_entity of a non-entity"));
        };
        let (generics, ports, estmts) = (generics.clone(), ports.clone(), stmts.clone());
        let UnitHir::Architecture { stmts, .. } = &self.units[arch].hir else {
            return Err(self.ice(at, "instantiate_entity of a non-architecture"));
        };
        let astmts = stmts.clone();
        if generics.len() != actuals.len() {
            return Err(self.ice(at, "generic actuals do not match the entity's generics"));
        }
        let escope = self.units[entity].scope;
        let ascope = self.units[arch].scope;
        let actx = self.units[arch].context;
        if self.scopes[ascope].parent != Some(actx) || self.scopes[actx].parent != Some(escope) {
            return Err(self.ice(at, "architecture region outside its entity's"));
        }
        let mangled = self.entity_mangled(entity, actuals);
        let ne = self.clone_unit(entity, mangled.clone());
        let na = self.clone_unit(arch, format!("{mangled}.{}", self.units[arch].name.key));
        let (ed, ad) = (self.units[ne].decl, self.units[na].decl);
        let nes = self.add_scope(
            self.scopes[escope].kind,
            self.scopes[escope].parent,
            Some(ed),
        );
        let mut r = Remap::new(escope, nes);
        r.bodies = true;
        r.decls.insert(self.units[entity].decl, ed);
        r.decls.insert(self.units[arch].decl, ad);
        if !self.seed_generics(&mut r, nes, escope, &generics, actuals) {
            return Err(Reported);
        }
        self.fill_scope(&mut r, escope, nes);
        let nas = self.r_scope(&mut r, ascope);
        let nactx = self.r_scope(&mut r, actx);
        let consts: Vec<DeclId> = generics
            .iter()
            .copied()
            .filter(|&g| matches!(self.decls[g].kind, DeclKind::Object(_)))
            .collect();
        let generics = consts.iter().map(|&g| self.r_decl(&mut r, g)).collect();
        let ports = ports.iter().map(|&p| self.r_decl(&mut r, p)).collect();
        let estmts = self.r_stmts(&mut r, &estmts);
        let astmts = self.r_stmts(&mut r, &astmts);
        self.fix_bodies(&mut r);
        self.clone_specs(&mut r, ascope, false, at)?;
        if r.failed {
            return Err(Reported);
        }
        self.units[ne].scope = nes;
        self.units[ne].hir = UnitHir::Entity {
            generics,
            ports,
            stmts: estmts,
        };
        self.units[na].scope = nas;
        self.units[na].context = nactx;
        self.units[na].hir = UnitHir::Architecture {
            entity: ne,
            stmts: astmts,
        };
        self.generic.entities.insert(key, (ne, na));
        Ok((ne, na))
    }

    /// `work.e[t=>std.standard.integer,f=>work.p.f(...)]`: the entity's mangled name
    /// with the mangled actual of each generic type, subprogram and package (5.3).
    fn entity_mangled(&self, entity: UnitId, actuals: &[Option<GenericActual>]) -> String {
        let UnitHir::Entity { generics, .. } = &self.units[entity].hir else {
            return String::new();
        };
        let parts: Vec<String> = generics
            .iter()
            .zip(actuals)
            .filter_map(|(&g, a)| {
                let v = match (*a)? {
                    GenericActual::Subtype(st) => crate::mangle::type_name(self, self.base(st)),
                    GenericActual::Subprogram(d) => match self.decls[d].kind.subprogram() {
                        Some(s) => s.mangled.to_string(),
                        None => self.decls[d].name.key().to_string(),
                    },
                    GenericActual::Package(i) => self.insts[i].mangled.to_string(),
                };
                Some(format!("{}=>{v}", self.decls[g].name.key()))
            })
            .collect();
        format!(
            "{}[{}]",
            crate::mangle::region(self, self.units[entity].scope),
            parts.join(",")
        )
    }

    /// A new unit like `u` with its own `DeclKind::Unit` entity; scope and HIR follow.
    fn clone_unit(&mut self, u: UnitId, mangled: String) -> UnitId {
        let o = &self.units[u];
        let unit = Unit {
            lib: o.lib,
            key: o.key,
            kind: o.kind,
            name: o.name,
            decl: o.decl,
            scope: o.scope,
            context: o.context,
            deps: o.deps.clone(),
            refs: Vec::new(),
            file: o.file,
            span: o.span,
            rev: o.rev,
            hir: UnitHir::Pending,
        };
        let n = self.units.push(unit);
        let od = &self.decls[self.units[u].decl];
        let d = self.decls.push(Decl {
            name: od.name,
            kind: DeclKind::Unit(n),
            scope: od.scope,
            span: od.span,
            implicit: od.implicit,
            since: od.since,
        });
        self.units[n].decl = d;
        self.generic.unit_names.insert(n, Symbol::intern(&mangled));
        n
    }

    /// Seed `r` with the actuals of `formals` (one per formal, `None` for generic
    /// constants) declared in `from`, cloned into `to`. False if one failed (reported).
    fn seed_generics(
        &mut self,
        r: &mut Remap,
        to: ScopeId,
        from: ScopeId,
        formals: &[DeclId],
        actuals: &[Option<GenericActual>],
    ) -> bool {
        let mut map = GenericMap::default();
        let mut failed = false;
        for (&f, a) in formals.iter().zip(actuals) {
            let span = self.decls[f].span;
            match (&self.decls[f].kind, a) {
                (DeclKind::Object(_), None) => {}
                (DeclKind::GenericType(t), Some(GenericActual::Subtype(st))) => {
                    let (t, st) = (*t, *st);
                    failed |= self.check_class(t, st, span, r).is_err()
                        || self
                            .seed_type(None, to, from, f, t, st, span, r, &mut map)
                            .is_err();
                }
                (DeclKind::GenericSubprogram(_), Some(GenericActual::Subprogram(d))) => {
                    r.decls.insert(f, *d);
                }
                (DeclKind::GenericPackage(fi), Some(GenericActual::Package(i))) => {
                    let (fi, i) = (*fi, *i);
                    if let Some(m) = self.generic.maps.get(&i) {
                        r.merge(&m.clone());
                    }
                    r.scopes.insert(self.insts[fi].scope, self.insts[i].scope);
                    let name = self.decls[f].name;
                    let new = self.new_decl(to, name, DeclKind::PackageInst(i), span);
                    r.decls.insert(f, new);
                }
                (DeclKind::Error, _) => failed = true,
                _ => {
                    failed = true;
                    self.ice(span, "generic actual of the wrong kind");
                }
            }
        }
        !failed
    }

    /// Clone the component configuration specifications of `root` and the regions inside
    /// it (keyed by region, naming statements, both renewed by the clone). `moved`: the
    /// originals are no longer reachable (a block instantiated in place).
    fn clone_specs(
        &mut self,
        r: &mut Remap,
        root: ScopeId,
        moved: bool,
        at: Span,
    ) -> Result<(), Reported> {
        let mut keys: Vec<ScopeId> = self
            .config_specs
            .keys()
            .copied()
            .filter(|&s| s == root || self.inside(s, root))
            .collect();
        keys.sort();
        for k in keys {
            let Some(&nk) = r.scopes.get(&k) else {
                return Err(self.ice(at, "configuration specification in a region not cloned"));
            };
            let specs = self.config_specs.remove(&k).unwrap_or_default();
            let mut out = Vec::with_capacity(specs.len());
            for c in &specs {
                if c.block.is_some() {
                    return Err(self.ice(c.span, "block configuration in a specification"));
                }
                let instances = match &c.instances {
                    InstanceSel::Labels(l) => InstanceSel::Labels(
                        l.iter().map(|s| *r.stmts.get(s).unwrap_or(s)).collect(),
                    ),
                    InstanceSel::All => InstanceSel::All,
                    InstanceSel::Others => InstanceSel::Others,
                };
                let binding = c.binding.as_ref().map(|b| Binding {
                    aspect: b.aspect.as_ref().map(|a| match a {
                        EntityAspect::Entity { unit, arch } => EntityAspect::Entity {
                            unit: *unit,
                            arch: *arch,
                        },
                        EntityAspect::Configuration(u) => EntityAspect::Configuration(*u),
                        EntityAspect::Open => EntityAspect::Open,
                    }),
                    generic_map: self.r_args(r, &b.generic_map),
                    port_map: self.r_args(r, &b.port_map),
                });
                out.push(ComponentConfig {
                    instances,
                    component: self.r_decl(r, c.component),
                    binding,
                    block: None,
                    span: c.span,
                });
            }
            if !moved {
                self.config_specs.insert(k, specs);
            }
            self.config_specs.edit(nk, |v| v.extend(out));
        }
        Ok(())
    }

    /// Whether `d` is a generic type, subprogram or package.
    pub fn is_non_constant_generic(&self, d: DeclId) -> bool {
        matches!(
            self.decls[d].kind,
            DeclKind::GenericType(_) | DeclKind::GenericSubprogram(_) | DeclKind::GenericPackage(_)
        )
    }

    /// The clone of component `comp` for these actuals of its generics (`None` for
    /// generic constants), with the formals replaced by the actuals: its generic list
    /// keeps only the generic constants, in order, its ports are in order. Elab elaborates
    /// the clone's generics and ports and binds the instance (#222).
    pub fn instantiate_component(
        &mut self,
        comp: DeclId,
        actuals: &[Option<GenericActual>],
        at: Span,
    ) -> Result<DeclId, Reported> {
        let key = (comp, actuals.to_vec().into_boxed_slice());
        if let Some(&c) = self.generic.components.get(&key) {
            return Ok(c);
        }
        let DeclKind::Component(c) = &self.decls[comp].kind else {
            return Err(self.ice(at, "instantiate_component of a non-component"));
        };
        let (cscope, generics, ports) = (c.scope, c.generics.clone(), c.ports.clone());
        if generics.len() != actuals.len() {
            return Err(self.ice(at, "generic actuals do not match the component's generics"));
        }
        let (name, dscope, span) = (
            self.decls[comp].name,
            self.decls[comp].scope,
            self.decls[comp].span,
        );
        let nd = self.new_decl(dscope, name, DeclKind::Error, span);
        let ncs = self.add_scope(
            self.scopes[cscope].kind,
            self.scopes[cscope].parent,
            Some(nd),
        );
        let mut r = Remap::new(cscope, ncs);
        r.decls.insert(comp, nd);
        if !self.seed_generics(&mut r, ncs, cscope, &generics, actuals) {
            return Err(Reported);
        }
        self.fill_scope(&mut r, cscope, ncs);
        let consts: Vec<DeclId> = generics
            .iter()
            .copied()
            .filter(|&g| !self.is_non_constant_generic(g))
            .collect();
        let generics = consts.iter().map(|&g| self.r_decl(&mut r, g)).collect();
        let ports = ports.iter().map(|&p| self.r_decl(&mut r, p)).collect();
        if r.failed {
            return Err(Reported);
        }
        self.decls[nd].kind = DeclKind::Component(Component {
            scope: ncs,
            generics,
            ports,
        });
        self.generic.components.insert(key, nd);
        Ok(nd)
    }

    /// Instantiate the blocks with generic types, subprograms or packages of architecture
    /// `unit`, innermost first, now that its configuration specifications are resolved
    /// (`architecture_body`). A failure is reported.
    pub(crate) fn instantiate_blocks(&mut self, unit: UnitId) {
        let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.generic.blocks)
            .into_iter()
            .partition(|(u, _)| *u == unit);
        self.generic.blocks = rest;
        for (_, st) in mine {
            let _ = self.instantiate_block(st);
        }
    }

    /// Replace block statement `st` with its generic types, subprograms and packages by a
    /// clone with the formals replaced by the actuals of its generic map (one generic map
    /// per block, so in place). Its generics and generic map keep the constants only.
    fn instantiate_block(&mut self, st: StmtId) -> Result<(), Reported> {
        let at = self.stmts[st].span;
        let StmtKind::Block {
            scope: bscope,
            guard,
            generics,
            generic_map,
            ports,
            port_map,
            body,
        } = &self.stmts[st].kind
        else {
            return Err(self.ice(at, "instantiate_block of a non-block"));
        };
        let (bscope, guard) = (*bscope, *guard);
        let (generics, generic_map, ports) = (generics.clone(), generic_map.clone(), ports.clone());
        let (port_map, body) = (port_map.clone(), body.clone());
        let mut actuals = Vec::with_capacity(generics.len());
        for (i, &g) in generics.iter().enumerate() {
            if !self.is_non_constant_generic(g) {
                actuals.push(None);
                continue;
            }
            let a = generic_map.iter().find(|a| a.formal as usize == i);
            actuals.push(Some(match a.map(|a| &a.actual) {
                Some(Actual::Subtype(x)) => GenericActual::Subtype(*x),
                Some(Actual::Subprogram(x)) => GenericActual::Subprogram(*x),
                Some(Actual::Package(x)) => GenericActual::Package(*x),
                _ => return Err(self.ice(at, "block generic without an actual")),
            }));
        }
        let nbs = self.add_scope(
            self.scopes[bscope].kind,
            self.scopes[bscope].parent,
            self.scopes[bscope].owner,
        );
        let mut r = Remap::new(bscope, nbs);
        r.bodies = true;
        if !self.seed_generics(&mut r, nbs, bscope, &generics, &actuals) {
            return Err(Reported);
        }
        self.fill_scope(&mut r, bscope, nbs);
        let mut index = vec![None; generics.len()];
        let mut consts = Vec::new();
        for (i, &g) in generics.iter().enumerate() {
            if actuals[i].is_none() {
                index[i] = Some(consts.len() as u16);
                consts.push(self.r_decl(&mut r, g));
            }
        }
        let kept: Vec<Arg> = generic_map
            .iter()
            .filter_map(|a| {
                let formal = index.get(a.formal as usize).copied().flatten()?;
                Some(Arg {
                    formal,
                    ..a.clone()
                })
            })
            .collect();
        let kind = StmtKind::Block {
            scope: nbs,
            guard: guard.map(|g| self.r_decl(&mut r, g)),
            generics: consts.into(),
            generic_map: self.r_args(&mut r, &kept),
            ports: ports.iter().map(|&p| self.r_decl(&mut r, p)).collect(),
            port_map: self.r_args(&mut r, &port_map),
            body: self.r_stmts(&mut r, &body),
        };
        self.fix_bodies(&mut r);
        self.clone_specs(&mut r, bscope, true, at)?;
        if r.failed {
            return Err(Reported);
        }
        self.stmts[st].kind = kind;
        Ok(())
    }

    /// Whether `s` lies inside region `outer`.
    fn inside(&self, mut s: crate::ids::ScopeId, outer: crate::ids::ScopeId) -> bool {
        while let Some(p) = self.scopes[s].parent {
            if p == outer {
                return true;
            }
            s = p;
        }
        false
    }
}
