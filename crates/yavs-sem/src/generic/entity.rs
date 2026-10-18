//! Design entities with generic types, subprograms or packages (2008 6.5.6): one clone of
//! the entity and architecture per distinct map of those actuals, with the formals replaced
//! by the actuals (5.1, 5.3). Generic constants stay generics of the clone: their values
//! are instance data that elab evaluates per instance.

use yavs_diag::Span;
use yavs_syntax::Symbol;

use super::{GenericMap, Remap};
use crate::db::{Db, Reported};
use crate::decl::{Decl, DeclKind};
use crate::hir::UnitHir;
use crate::ids::{DeclId, InstId, SubtypeId, UnitId};
use crate::loader::Unit;

/// The actual of a generic type, subprogram or package of a design entity, as the HIR
/// generic map gives it (`Actual::Subtype`, `Actual::Subprogram`, `Actual::Package`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
        // Component configuration specifications are keyed by region and statement, which
        // the clone renews.
        let specs = self
            .config_specs
            .keys()
            .any(|&s| s == ascope || self.inside(s, ascope));
        if specs {
            return Err(self.unsupported(
                at,
                "configuration specifications in an entity with generic types, subprograms or packages",
                "S4",
            ));
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
        let mut map = GenericMap::default();
        let mut failed = false;
        for (&f, a) in generics.iter().zip(actuals) {
            let span = self.decls[f].span;
            match (&self.decls[f].kind, a) {
                (DeclKind::Object(_), None) => {}
                (DeclKind::GenericType(t), Some(GenericActual::Subtype(st))) => {
                    let (t, st) = (*t, *st);
                    failed |= self
                        .seed_type(None, nes, escope, f, t, st, span, &mut r, &mut map)
                        .is_err();
                }
                (DeclKind::GenericSubprogram(_), Some(GenericActual::Subprogram(d))) => {
                    r.decls.insert(f, *d);
                    map.subprograms.insert(f, *d);
                }
                (DeclKind::GenericPackage(fi), Some(GenericActual::Package(i))) => {
                    let (fi, i) = (*fi, *i);
                    if let Some(m) = self.generic.maps.get(&i) {
                        r.merge(&m.clone());
                    }
                    r.scopes.insert(self.insts[fi].scope, self.insts[i].scope);
                    let name = self.decls[f].name;
                    let new = self.new_decl(nes, name, DeclKind::PackageInst(i), span);
                    r.decls.insert(f, new);
                    map.packages.insert(f, i);
                }
                (DeclKind::Error, _) => failed = true,
                _ => {
                    failed = true;
                    self.ice(span, "generic actual of the wrong kind");
                }
            }
        }
        if failed {
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
