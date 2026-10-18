//! The elaborator: instances, generics, generates, blocks, binding, ports and processes
//! (ir.md 4, steps 1 to 10), depth-first in statement order.

use std::collections::HashMap;

use rsim_diag::{Diagnostic, Span};
use rsim_kernel::{Elem, PortMode, Range};
use rsim_sem::decl::{DeclKind, ObjClass, ObjRole};
use rsim_sem::hir::{
    Actual, Arg, BlockConfig, BlockTarget, Choice, ConfigItem, EntityAspect, ExprKind, GenSelect,
    InstTarget, InstanceSel, Lsp, LspStep, Mode, ObjRoot, StmtKind, UnitHir,
};
use rsim_sem::loader::UnitKind;
use rsim_sem::{Db, DeclId, ExprId, StmtId, UnitId};
use rsim_syntax::Symbol;
use rsim_syntax::ast::Ident;

use crate::eval::{Model, Val, leaf_values, parse_override};
use crate::hier::{
    Case, DriverDecl, Leaf, LeafId, LeafRes, ObjId, ObjKind, PortEdge, ProcIdx, ScopeId, ScopeKind,
    Sel, Shape, SigObj,
};
use crate::{ElabConfig, Elaborated, ProcInst, Top, codes};

type R<T> = Result<T, Box<Diagnostic>>;

/// How one formal is associated (from a HIR map, or a default binding by name).
enum A {
    Expr(ExprId),
    /// A component local of the same name (default binding, LRM 7.3.3).
    Local(DeclId),
    Open,
    Default,
    Parts(Vec<(Vec<LspStep>, ExprId)>),
    Conv(ExprId),
}

fn assocs(args: &[Arg], n: usize) -> Vec<A> {
    let mut v: Vec<A> = (0..n).map(|_| A::Default).collect();
    for a in args {
        let x = match &a.actual {
            Actual::Expr(e) => A::Expr(*e),
            Actual::Default => A::Default,
            Actual::Open => A::Open,
            Actual::Individual(p) => A::Parts(p.iter().map(|(s, e)| (s.to_vec(), *e)).collect()),
            Actual::Converted { actual, .. } => A::Conv(*actual),
        };
        if let Some(slot) = v.get_mut(a.formal as usize) {
            *slot = x;
        }
    }
    v
}

/// A block configuration inside the HIR, as a path (the HIR stays borrowed only briefly).
#[derive(Clone)]
enum CfgRoot {
    Unit(UnitId),
    Spec(rsim_sem::ScopeId, usize),
}

#[derive(Clone)]
struct CfgPath {
    root: CfgRoot,
    steps: Vec<usize>,
}

fn block_cfg<'a>(db: &'a Db, p: &CfgPath) -> Option<&'a BlockConfig> {
    let mut b = match p.root {
        CfgRoot::Unit(u) => match &db.units[u].hir {
            UnitHir::Configuration(b) => b,
            _ => return None,
        },
        CfgRoot::Spec(r, i) => db.config_specs.get(&r)?.get(i)?.block.as_ref()?,
    };
    for &s in &p.steps {
        b = match b.items.get(s)? {
            ConfigItem::Block(b) => b,
            ConfigItem::Component(c) => c.block.as_ref()?,
        };
    }
    Some(b)
}

struct Bound {
    entity: UnitId,
    arch: Option<Symbol>,
    gmap: Vec<A>,
    pmap: Vec<A>,
    cfg: Option<CfgPath>,
}

pub(crate) struct Elab<'d> {
    db: &'d mut Db,
    m: Model,
    procs: Vec<ProcInst>,
    errors: Vec<Diagnostic>,
    warnings: Vec<Diagnostic>,
    cfg: ElabConfig,
}

pub(crate) fn find_top(db: &mut Db, work: &str, name: &str) -> Result<Top, Vec<Diagnostic>> {
    let (prim, arch) = match name.split_once('.') {
        Some((p, a)) => (p, Some(a)),
        None => (name, None),
    };
    let fail = |db: &mut Db, d: Diagnostic| {
        let mut v = db.take_diagnostics();
        v.push(d);
        v
    };
    let Ok(u) = db.load_primary(work, prim) else {
        let d = Diagnostic::error(
            codes::TOP_NOT_FOUND,
            format!("no top-level unit `{prim}` in library `{work}`"),
        )
        .with_help("TOP is ENTITY, ENTITY.ARCH or CONFIGURATION, analyzed into the work library");
        return Err(fail(db, d));
    };
    match db.units[u].kind {
        UnitKind::Entity => {
            let a = match arch {
                Some(a) => Symbol::intern(&rsim_lib::fold(a)),
                None => match db.last_architecture(u) {
                    Some(a) => a,
                    None => {
                        let d = Diagnostic::error(
                            codes::NO_ARCHITECTURE,
                            format!("entity `{}` has no architecture", db.units[u].name.text),
                        )
                        .with_primary(db.units[u].name.span, "entity declared here")
                        .with_help("analyze an architecture of it into the work library");
                        return Err(fail(db, d));
                    }
                },
            };
            let lib = db.units[u].lib;
            let key = db.units[u].name.key;
            match db.load_secondary(lib, key, a, None) {
                Ok(au) => Ok(Top {
                    entity: u,
                    arch: au,
                    config: None,
                }),
                Err(_) => {
                    let d = Diagnostic::error(
                        codes::NO_ARCHITECTURE,
                        format!(
                            "entity `{}` has no architecture `{a}`",
                            db.units[u].name.text
                        ),
                    )
                    .with_primary(db.units[u].name.span, "entity declared here");
                    Err(fail(db, d))
                }
            }
        }
        UnitKind::Configuration if arch.is_none() => match &db.units[u].hir {
            UnitHir::Configuration(BlockConfig {
                target: BlockTarget::Architecture(a),
                ..
            }) => {
                let a = *a;
                let UnitHir::Architecture { entity, .. } = db.units[a].hir else {
                    return Err(fail(db, ice(Span::default(), "configuration target")));
                };
                Ok(Top {
                    entity,
                    arch: a,
                    config: Some(u),
                })
            }
            _ => Err(fail(
                db,
                ice(db.units[u].span, "configuration without target"),
            )),
        },
        _ => {
            let d = Diagnostic::error(
                codes::TOP_NOT_FOUND,
                format!(
                    "`{}` is not an entity or a configuration",
                    db.units[u].name.text
                ),
            )
            .with_primary(db.units[u].name.span, "declared here")
            .with_help("TOP is ENTITY, ENTITY.ARCH or CONFIGURATION");
            Err(fail(db, d))
        }
    }
}

fn ice(span: Span, what: &str) -> Diagnostic {
    Diagnostic::error(codes::INTERNAL, format!("internal error: {what}"))
        .with_primary(span, "while elaborating this")
        .with_note("this is a bug in rsim")
        .with_help("please report it with the source that triggers it")
}

fn unsupported(span: Span, what: &str, when: &str) -> Box<Diagnostic> {
    Box::new(
        Diagnostic::error(
            codes::NOT_IMPLEMENTED,
            format!("{what} is not implemented yet in rsim's elaboration"),
        )
        .with_primary(span, "not supported yet")
        .with_note(format!(
            "planned for {when}; this is a limitation of rsim, not an error in the design"
        )),
    )
}

pub(crate) fn elaborate(
    db: &mut Db,
    top: Top,
    overrides: &[(String, String)],
    cfg: &ElabConfig,
) -> Result<Elaborated, Vec<Diagnostic>> {
    let mut e = Elab {
        db,
        m: Model::default(),
        procs: Vec::new(),
        errors: Vec::new(),
        warnings: Vec::new(),
        cfg: cfg.clone(),
    };
    let name = e.db.units[top.entity].name;
    let cfgp = top.config.map(|u| CfgPath {
        root: CfgRoot::Unit(u),
        steps: Vec::new(),
    });
    let n_gen = match &e.db.units[top.entity].hir {
        UnitHir::Entity { generics, .. } => generics.len(),
        _ => 0,
    };
    let n_port = match &e.db.units[top.entity].hir {
        UnitHir::Entity { ports, .. } => ports.len(),
        _ => 0,
    };
    let gens = (0..n_gen).map(|_| A::Default).collect();
    let ports = (0..n_port).map(|_| A::Open).collect();
    let r = e.instance(
        None,
        name,
        top.entity,
        top.arch,
        gens,
        None,
        ports,
        None,
        cfgp,
        overrides,
        0,
        e.db.units[top.entity].name.span,
    );
    if let Err(d) = r {
        e.errors.push(*d);
    }
    if e.errors.is_empty() {
        e.check_sources();
    }
    if !e.errors.is_empty() {
        let mut v = e.db.take_diagnostics();
        v.append(&mut e.warnings);
        v.append(&mut e.errors);
        return Err(v);
    }
    Ok(Elaborated {
        top,
        hier: e.m.hier,
        nets: e.m.nets,
        procs: e.procs,
        model_vals: e.m.vals,
        sigs: e.m.sigs,
        warnings: e.warnings,
        ir: None,
    })
}

impl Elab<'_> {
    /// Attach the instance path to an error (fail clearly: where in the hierarchy).
    fn ctx(&self, s: ScopeId, mut d: Box<Diagnostic>) -> Box<Diagnostic> {
        if !d.notes.iter().any(|n| n.starts_with("in instance")) {
            d = Box::new(d.with_note(format!(
                "in instance {}",
                self.m.hier.path_name(s, Case::Original)
            )));
        }
        d
    }

    /// Load the units Db needs on demand; their diagnostics become ours.
    fn load_arch(&mut self, entity: UnitId, arch: Option<Symbol>, at: Span) -> R<UnitId> {
        let a = match arch.or_else(|| self.db.last_architecture(entity)) {
            Some(a) => a,
            None => {
                return Err(Box::new(
                    Diagnostic::error(
                        codes::NO_ARCHITECTURE,
                        format!(
                            "entity `{}` has no architecture",
                            self.db.units[entity].name.text
                        ),
                    )
                    .with_primary(at, "instantiated here")
                    .with_secondary(self.db.units[entity].name.span, "entity declared here"),
                ));
            }
        };
        let (lib, key) = (self.db.units[entity].lib, self.db.units[entity].name.key);
        match self.db.load_secondary(lib, key, a, Some(at)) {
            Ok(u) => Ok(u),
            Err(_) => {
                let mut ds = self.db.take_diagnostics();
                let last = ds
                    .pop()
                    .map(Box::new)
                    .unwrap_or_else(|| Box::new(ice(at, "unit load failed without a diagnostic")));
                self.errors.extend(ds);
                Err(last)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn instance(
        &mut self,
        parent: Option<ScopeId>,
        label: Ident,
        entity: UnitId,
        arch: UnitId,
        gens: Vec<A>,
        gscope: Option<ScopeId>,
        ports: Vec<A>,
        pscope: Option<ScopeId>,
        cfg: Option<CfgPath>,
        overrides: &[(String, String)],
        depth: u32,
        at: Span,
    ) -> R<ScopeId> {
        let s = self
            .m
            .hier
            .add(parent, ScopeKind::Instance { entity, arch }, label);
        if depth > self.cfg.max_depth {
            return Err(self.ctx(
                s,
                Box::new(
                    Diagnostic::error(
                        codes::TOO_DEEP,
                        format!("instances nested deeper than {}", self.cfg.max_depth),
                    )
                    .with_primary(at, "instantiated here")
                    .with_help("a recursive design must stop at a generate condition"),
                ),
            ));
        }
        let (generics, eports, estmts) = match &self.db.units[entity].hir {
            UnitHir::Entity {
                generics,
                ports,
                stmts,
            } => (generics.to_vec(), ports.to_vec(), stmts.to_vec()),
            _ => return Err(Box::new(ice(at, "instance of a non-entity"))),
        };
        // 3. Generics: actuals in the parent, then overrides, then defaults.
        for (i, &g) in generics.iter().enumerate() {
            let r = self.generic(
                s,
                g,
                gens.get(i).unwrap_or(&A::Default),
                gscope,
                overrides,
                at,
            );
            if let Err(d) = r {
                return Err(self.ctx(s, d));
            }
        }
        if parent.is_none() {
            for (name, _) in overrides {
                let key = rsim_lib::fold(name);
                if !generics
                    .iter()
                    .any(|&g| self.db.decls[g].name.key().as_str() == key)
                {
                    self.warnings.push(
                        Diagnostic::warning(
                            codes::UNKNOWN_OVERRIDE,
                            format!(
                                "`-g{name}`: top level `{}` has no generic `{name}`",
                                label.text
                            ),
                        )
                        .with_primary(label.span, "top-level entity"),
                    );
                }
            }
        }
        // 7. Ports.
        for (i, &p) in eports.iter().enumerate() {
            let r = self.port(s, p, ports.get(i).unwrap_or(&A::Open), pscope, at);
            if let Err(d) = r {
                return Err(self.ctx(s, d));
            }
        }
        // 4. Declarations of the entity, then of the architecture.
        let escope = self.db.units[entity].scope;
        let ascope = self.db.units[arch].scope;
        for region in [escope, ascope] {
            if let Err(d) = self.decls(s, region) {
                return Err(self.ctx(s, d));
            }
        }
        let UnitHir::Architecture { stmts, .. } = &self.db.units[arch].hir else {
            return Err(Box::new(ice(at, "architecture without HIR")));
        };
        let astmts = stmts.to_vec();
        let mut counter = 0;
        self.stmts(s, &estmts, None, &mut counter, depth)?;
        self.stmts(s, &astmts, cfg.as_ref(), &mut counter, depth)?;
        Ok(s)
    }

    fn generic(
        &mut self,
        s: ScopeId,
        g: DeclId,
        a: &A,
        gscope: Option<ScopeId>,
        overrides: &[(String, String)],
        at: Span,
    ) -> R<()> {
        let db: &Db = self.db;
        let decl = &db.decls[g];
        let DeclKind::Object(o) = &decl.kind else {
            return Err(unsupported(
                decl.span,
                "generic types, subprograms and packages",
                "slice 3 (with rsim-lower)",
            ));
        };
        let key = decl.name.key().as_str().to_string();
        let ov = overrides
            .iter()
            .rev()
            .find(|(n, _)| rsim_lib::fold(n) == key);
        let v = match (ov, a) {
            (Some((n, text)), _) if gscope.is_none() => parse_override(&mut self.m, db, g, text, s)
                .map_err(|m| {
                    Box::new(
                        Diagnostic::error(
                            codes::BAD_OVERRIDE,
                            format!("invalid value for generic `{}`: `-g{n}={text}`", decl.name),
                        )
                        .with_primary(decl.span, "generic declared here")
                        .with_note(m),
                    )
                })?,
            (_, A::Expr(e)) => self.m.eval(db, *e, gscope)?,
            (_, A::Local(l)) => self.m.value_of(db, *l, gscope, at)?,
            (_, A::Default | A::Open) => match o.default {
                Some(d) => self.m.eval(db, d, Some(s))?,
                None => {
                    return Err(Box::new(
                        Diagnostic::error(
                            codes::GENERIC_NO_VALUE,
                            format!("generic `{}` has no value", decl.name),
                        )
                        .with_primary(at, "instantiated here without a value for it")
                        .with_secondary(decl.span, "generic declared here without a default")
                        .with_help(if gscope.is_none() {
                            format!("give it one with `-g{}=VALUE`", decl.name)
                        } else {
                            "associate it in the generic map".to_string()
                        }),
                    ));
                }
            },
            (_, A::Parts(_) | A::Conv(_)) => {
                return Err(unsupported(
                    decl.span,
                    "individual or converted generic associations",
                    "M3",
                ));
            }
        };
        // Scalar generic values must lie in the generic's subtype (LRM 6.5.6.2).
        if db.types[db.base(o.subtype)].kind.is_scalar()
            && let Some(x) = v.as_int()
            && let Ok(Some(Shape::Scalar { .. })) = self.m.shape(db, o.subtype, Some(s), decl.span)
        {
            let b = self.m.m_scalar(self.db, o.subtype, s, decl.span)?;
            if b.pos(x).is_none() {
                return Err(Box::new(
                    Diagnostic::error(
                        codes::GENERIC_RANGE,
                        format!(
                            "value {x} of generic `{}` is outside its subtype {} {} {}",
                            decl.name,
                            b.left,
                            if b.dir == rsim_sem::hir::Direction::To {
                                "to"
                            } else {
                                "downto"
                            },
                            b.right
                        ),
                    )
                    .with_primary(at, "instantiated here")
                    .with_secondary(decl.span, "generic declared here"),
                ));
            }
        }
        self.m.set(s, g, v);
        Ok(())
    }

    /// Signal declarations of a declarative region (constants are evaluated on demand).
    fn decls(&mut self, s: ScopeId, region: rsim_sem::ScopeId) -> R<()> {
        let order = self.db.scopes[region].order.clone();
        for d in order {
            let db: &Db = self.db;
            let decl = &db.decls[d];
            let DeclKind::Object(o) = &decl.kind else {
                continue;
            };
            match (o.class, &o.role) {
                (ObjClass::Signal, ObjRole::Plain) => {
                    let span = decl.span;
                    let Some(shape) = self.m.shape(db, o.subtype, Some(s), span)? else {
                        return Err(Box::new(ice(span, "signal of an unconstrained subtype")));
                    };
                    if o.signal_kind.is_some() {
                        return Err(unsupported(
                            span,
                            "guarded signals (`bus`, `register`)",
                            "M3",
                        ));
                    }
                    let init = match o.default {
                        Some(e) => Some(self.m.eval(db, e, Some(s))?),
                        None => None,
                    };
                    self.m
                        .add_obj(self.db, s, d, ObjKind::Signal, shape, init.as_ref(), span)?;
                }
                (ObjClass::Signal, ObjRole::ImplicitSignal | ObjRole::Guard) => {
                    return Err(unsupported(
                        decl.span,
                        "implicit signals ('stable, 'quiet, 'delayed, 'transaction, GUARD)",
                        "slice 4",
                    ));
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Ports (rule 3): each formal is its own signal; its association becomes port edges.
    fn port(&mut self, s: ScopeId, p: DeclId, a: &A, ascope: Option<ScopeId>, at: Span) -> R<()> {
        let db: &Db = self.db;
        let decl = &db.decls[p];
        let span = decl.span;
        let DeclKind::Object(o) = &decl.kind else {
            return Err(Box::new(ice(span, "port is not an object")));
        };
        let component = matches!(self.m.hier.scope(s).kind, ScopeKind::Component { .. });
        let mode = match o.mode {
            Some(Mode::In) => PortMode::In,
            Some(Mode::Out) => PortMode::Out,
            Some(Mode::Inout) => PortMode::Inout,
            Some(Mode::Buffer) => PortMode::Buffer,
            Some(Mode::Linkage) => return Err(unsupported(span, "linkage ports", "M3")),
            Some(Mode::View { .. }) | None => {
                return Err(unsupported(span, "mode view ports", "M3"));
            }
        };
        let kind = if component {
            ObjKind::ComponentPort(mode)
        } else {
            ObjKind::Port(mode)
        };
        let (subtype, default) = (o.subtype, o.default);
        let own = self.m.shape(db, subtype, Some(s), span)?;
        let init = match default {
            Some(e) => Some(self.m.eval(db, e, Some(s))?),
            None => None,
        };
        // The actual: a signal name, a value, or nothing.
        let actual = match (a, ascope) {
            (A::Expr(e), Some(sc)) => match self.m.name_ref(self.db, *e, sc)? {
                Some(n) => Some((n, db.exprs[*e].span)),
                None => {
                    // 2008 expression actual: a static value is the port's value.
                    let v = self.m.eval(db, *e, Some(sc)).map_err(|d| {
                        if d.code == codes::NOT_EVALUATED {
                            unsupported(
                                db.exprs[*e].span,
                                "non-static expressions as port actuals",
                                "slice 4 (implicit processes, rsim-lower)",
                            )
                        } else {
                            d
                        }
                    })?;
                    if mode != PortMode::In {
                        return Err(Box::new(ice(
                            db.exprs[*e].span,
                            "expression actual of a non-in port",
                        )));
                    }
                    let Some(shape) = own else {
                        return Err(unsupported(
                            db.exprs[*e].span,
                            "an expression actual for an unconstrained port",
                            "M3",
                        ));
                    };
                    self.m
                        .add_obj(self.db, s, p, kind, shape, Some(&v), db.exprs[*e].span)?;
                    return Ok(());
                }
            },
            (A::Local(l), Some(sc)) => match self.m.signal(sc, *l) {
                Some(o) => Some(((o, Vec::new()), at)),
                None => return Err(Box::new(ice(at, "component port without object"))),
            },
            (A::Conv(e), _) => {
                return Err(unsupported(
                    db.exprs[*e].span,
                    "conversion functions in port maps",
                    "slice 4 (with rsim-lower)",
                ));
            }
            (A::Parts(parts), Some(sc)) => {
                let Some(shape) = own else {
                    return Err(unsupported(
                        span,
                        "individual association of an unconstrained port",
                        "M3",
                    ));
                };
                let fo = self
                    .m
                    .add_obj(self.db, s, p, kind, shape, init.as_ref(), span)?;
                for (steps, e) in parts {
                    let espan = self.db.exprs[*e].span;
                    let fsels = self.m.steps(self.db, steps, sc, espan)?;
                    let (fl, _) = self.m.select(fo, &fsels, espan)?;
                    let Some((ao, asels)) = self.m.name_ref(self.db, *e, sc)? else {
                        return Err(unsupported(
                            espan,
                            "expression actuals in individual associations",
                            "slice 4",
                        ));
                    };
                    let (al, _) = self.m.select(ao, &asels, espan)?;
                    self.m.edges(&fl, &al, mode, espan)?;
                }
                return Ok(());
            }
            _ => None,
        };
        let Some(((ao, asels), aspan)) = actual else {
            // open or default
            let Some(shape) = own else {
                return Err(Box::new(
                    Diagnostic::error(
                        codes::PORT_UNCONSTRAINED,
                        format!("port `{}` is unconstrained and not associated", decl.name),
                    )
                    .with_primary(at, "instantiated here")
                    .with_secondary(span, "port declared here")
                    .with_help("associate it with a signal, or give it a constrained subtype"),
                ));
            };
            self.m
                .add_obj(self.db, s, p, kind, shape, init.as_ref(), span)?;
            return Ok(());
        };
        let (al, ashape) = self.m.select(ao, &asels, aspan)?;
        let shape = own.unwrap_or(ashape);
        let fo = self
            .m
            .add_obj(self.db, s, p, kind, shape, init.as_ref(), span)?;
        let fl = self.m.select(fo, &[], span)?.0;
        self.m.edges(&fl, &al, mode, aspan)
    }

    // -- statements ---------------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    fn stmts(
        &mut self,
        s: ScopeId,

        stmts: &[StmtId],
        cfg: Option<&CfgPath>,
        counter: &mut u32,
        depth: u32,
    ) -> R<()> {
        for &st in stmts {
            if let Err(d) = self.stmt(s, st, cfg, counter, depth) {
                return Err(self.ctx(s, d));
            }
        }
        Ok(())
    }

    fn stmt(
        &mut self,
        s: ScopeId,

        st: StmtId,
        cfg: Option<&CfgPath>,
        counter: &mut u32,
        depth: u32,
    ) -> R<()> {
        let db: &Db = self.db;
        let stmt = &db.stmts[st];
        let (label, span) = (stmt.label, stmt.span);
        let named = |db: &Db| -> R<Ident> {
            label.ok_or_else(|| Box::new(ice(db.stmts[st].span, "unlabeled statement")))
        };
        match &stmt.kind {
            StmtKind::Process {
                postponed, info, ..
            }
            | StmtKind::ConcSignalAssign {
                postponed, info, ..
            }
            | StmtKind::ConcProcCall {
                postponed, info, ..
            }
            | StmtKind::ConcAssert {
                postponed, info, ..
            } => {
                let name = label.unwrap_or_else(|| {
                    let n = Symbol::intern(&format!("_p{counter}"));
                    *counter += 1;
                    Ident {
                        text: n,
                        key: n,
                        span,
                    }
                });
                let (postponed, lsps) = (*postponed, info.drivers.clone());
                self.process(s, st, name, postponed, &lsps)
            }
            StmtKind::Block {
                scope: bscope,
                guard,
                generics,
                ports,
                body,
                ..
            } => {
                if guard.is_some() || !generics.is_empty() || !ports.is_empty() {
                    return Err(unsupported(
                        span,
                        "guarded blocks and blocks with generics or ports",
                        "M3",
                    ));
                }
                let (bscope, body) = (*bscope, body.to_vec());
                let b = self.m.hier.add(Some(s), ScopeKind::Block, named(db)?);
                let bcfg = cfg.and_then(|c| self.sub_cfg(c, st, None));
                self.decls(b, bscope)?;
                let mut c = 0;
                self.stmts(b, &body, bcfg.as_ref(), &mut c, depth)
            }
            StmtKind::ForGenerate {
                param,
                range,
                scope: gscope,
                body,
            } => {
                let (param, gscope, body) = (*param, *gscope, body.to_vec());
                let b = self.m.range(db, range, Some(s), span)?;
                let label = named(db)?;
                let n = b.len();
                for i in 0..n {
                    let v = match b.dir {
                        rsim_sem::hir::Direction::To => b.left + i as i64,
                        rsim_sem::hir::Direction::Downto => b.left - i as i64,
                    };
                    let db: &Db = self.db;
                    let val = match db.types[db.base(match &db.decls[param].kind {
                        DeclKind::Object(o) => o.subtype,
                        _ => return Err(Box::new(ice(span, "generate parameter"))),
                    })]
                    .kind
                    {
                        rsim_sem::types::TypeKind::Enum { .. } => Val::Enum(v as u32),
                        _ => Val::Int(v),
                    };
                    let image = image(db, param, &val);
                    let g = self.m.hier.add(
                        Some(s),
                        ScopeKind::ForGenerate {
                            param,
                            value: val.clone(),
                        },
                        label,
                    );
                    self.m.hier.scopes[g.index()].index = Some(image);
                    self.m.set(g, param, val.clone());
                    let gcfg = cfg.and_then(|c| self.sub_cfg(c, st, Some(&val)));
                    self.decls(g, gscope)?;
                    let mut c = 0;
                    self.stmts(g, &body, gcfg.as_ref(), &mut c, depth)?;
                }
                Ok(())
            }
            StmtKind::IfGenerate { alts } => {
                let label = named(db)?;
                let alts: Vec<_> = alts
                    .iter()
                    .map(|a| (a.cond, a.scope, a.body.to_vec(), a.label))
                    .collect();
                for (cond, gscope, body, alabel) in alts {
                    let take = match cond {
                        None => true,
                        Some(c) => {
                            let db: &Db = self.db;
                            self.m.eval(db, c, Some(s))?.as_int() == Some(1)
                        }
                    };
                    if take {
                        let alt = alabel.map(|l| l.key);
                        return self.gen_alt(s, st, label, gscope, &body, cfg, alt, depth);
                    }
                }
                Ok(())
            }
            StmtKind::CaseGenerate { expr, alts } => {
                let label = named(db)?;
                let v = self.m.eval(db, *expr, Some(s))?;
                let mut pick = None;
                'alts: for (cs, alt) in alts.iter() {
                    for ch in cs.choices.iter() {
                        let hit = match ch {
                            Choice::Others => true,
                            Choice::Expr(e) => self.m.eval(db, *e, Some(s))? == v,
                            Choice::Range(r) => {
                                let b = self.m.range(db, r, Some(s), span)?;
                                v.as_int().is_some_and(|x| b.pos(x).is_some())
                            }
                            Choice::Element(_) => false,
                        };
                        if hit {
                            pick = Some((alt.scope, alt.body.to_vec(), alt.label.map(|l| l.key)));
                            break 'alts;
                        }
                    }
                }
                match pick {
                    Some((gscope, body, alt)) => {
                        self.gen_alt(s, st, label, gscope, &body, cfg, alt, depth)
                    }
                    None => Ok(()),
                }
            }
            StmtKind::Instance {
                target,
                scope: iregion,
                generic_map,
                port_map,
            } => {
                let label = named(db)?;
                match target {
                    InstTarget::Entity { unit, arch } => {
                        let (unit, arch) = (*unit, arch.map(|a| a.key));
                        let (ng, np) = counts(db, unit);
                        let (g, p) = (assocs(generic_map, ng), assocs(port_map, np));
                        let a = self.load_arch(unit, arch, span)?;
                        self.instance(
                            Some(s),
                            label,
                            unit,
                            a,
                            g,
                            Some(s),
                            p,
                            Some(s),
                            None,
                            &[],
                            depth + 1,
                            span,
                        )?;
                        Ok(())
                    }
                    InstTarget::Configuration(cu) => {
                        let cu = *cu;
                        let Ok(top) = find_top_of_config(db, cu) else {
                            return Err(Box::new(ice(span, "configuration without architecture")));
                        };
                        let (ng, np) = counts(db, top.0);
                        let (g, p) = (assocs(generic_map, ng), assocs(port_map, np));
                        let c = CfgPath {
                            root: CfgRoot::Unit(cu),
                            steps: Vec::new(),
                        };
                        self.instance(
                            Some(s),
                            label,
                            top.0,
                            top.1,
                            g,
                            Some(s),
                            p,
                            Some(s),
                            Some(c),
                            &[],
                            depth + 1,
                            span,
                        )?;
                        Ok(())
                    }
                    InstTarget::Component(comp) => {
                        let comp = *comp;
                        let DeclKind::Component(c) = &db.decls[comp].kind else {
                            return Err(Box::new(ice(span, "component")));
                        };
                        let (cg, cp) = (c.generics.to_vec(), c.ports.to_vec());
                        let (g, p) = (assocs(generic_map, cg.len()), assocs(port_map, cp.len()));
                        self.component(
                            s, *iregion, st, label, comp, &cg, &cp, g, p, cfg, depth, span,
                        )
                    }
                }
            }
            _ => Err(Box::new(ice(
                span,
                "sequential statement in a concurrent region",
            ))),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn gen_alt(
        &mut self,
        s: ScopeId,
        st: StmtId,
        label: Ident,
        gscope: rsim_sem::ScopeId,
        body: &[StmtId],
        cfg: Option<&CfgPath>,
        alt: Option<Symbol>,
        depth: u32,
    ) -> R<()> {
        let g = self.m.hier.add(Some(s), ScopeKind::Generate, label);
        let gcfg = cfg.and_then(|c| self.sub_cfg_alt(c, st, alt));
        self.decls(g, gscope)?;
        let mut c = 0;
        self.stmts(g, body, gcfg.as_ref(), &mut c, depth)
    }

    /// The block configuration item for block or generate statement `st` (with the
    /// generate parameter value `val`).
    fn sub_cfg(&mut self, c: &CfgPath, st: StmtId, val: Option<&Val>) -> Option<CfgPath> {
        let db: &Db = self.db;
        let b = block_cfg(db, c)?;
        for (i, it) in b.items.iter().enumerate() {
            let ConfigItem::Block(bc) = it else { continue };
            let hit = match &bc.target {
                BlockTarget::Block(s) => *s == st,
                BlockTarget::Generate { stmt, select } => {
                    *stmt == st
                        && match (select, val) {
                            (None, _) => true,
                            (Some(GenSelect::Index(e)), Some(v)) => {
                                self.m.eval(db, *e, None).ok().as_ref() == Some(v)
                            }
                            (Some(GenSelect::Range(r)), Some(v)) => self
                                .m
                                .range(db, r, None, bc.span)
                                .ok()
                                .zip(v.as_int())
                                .is_some_and(|(b, x)| b.pos(x).is_some()),
                            _ => false,
                        }
                }
                BlockTarget::Architecture(_) => false,
            };
            if hit {
                let mut p = c.clone();
                p.steps.push(i);
                return Some(p);
            }
        }
        None
    }

    fn sub_cfg_alt(&mut self, c: &CfgPath, st: StmtId, alt: Option<Symbol>) -> Option<CfgPath> {
        let b = block_cfg(self.db, c)?;
        let i = b.items.iter().position(|it| {
            matches!(it, ConfigItem::Block(bc) if matches!(&bc.target,
            BlockTarget::Generate { stmt, select } if *stmt == st && match select {
                None => true,
                Some(GenSelect::Alternative(a)) => Some(*a) == alt,
                _ => false,
            }))
        })?;
        let mut p = c.clone();
        p.steps.push(i);
        Some(p)
    }

    /// Binding of component instance `st` (LRM 7.3.3): configuration declaration, then
    /// configuration specification, then default binding.
    fn binding(
        &mut self,
        region: rsim_sem::ScopeId,
        st: StmtId,
        comp: DeclId,
        cfg: Option<&CfgPath>,
        span: Span,
    ) -> R<Option<Bound>> {
        let db: &Db = self.db;
        let pick = |items: &mut dyn Iterator<Item = (usize, &rsim_sem::hir::ComponentConfig)>| {
            let items: Vec<_> = items.filter(|(_, c)| c.component == comp).collect();
            let named = |c: &rsim_sem::hir::ComponentConfig| matches!(&c.instances, InstanceSel::Labels(l) if l.contains(&st));
            let labelled_elsewhere = items.iter().any(|(_, c)| named(c));
            items
                .iter()
                .find(|(_, c)| named(c))
                .or_else(|| {
                    items
                        .iter()
                        .find(|(_, c)| matches!(c.instances, InstanceSel::All))
                })
                .or_else(|| {
                    items.iter().find(|(_, c)| {
                        matches!(c.instances, InstanceSel::Others) && !labelled_elsewhere
                    })
                })
                .map(|(i, _)| *i)
        };
        // Configuration declaration item, then specification in the region.
        let mut found: Option<CfgPath> = None;
        if let Some(c) = cfg
            && let Some(b) = block_cfg(db, c)
        {
            let mut it = b.items.iter().enumerate().filter_map(|(i, it)| match it {
                ConfigItem::Component(cc) => Some((i, cc)),
                _ => None,
            });
            if let Some(i) = pick(&mut it) {
                let mut p = c.clone();
                p.steps.push(i);
                found = Some(p);
            }
        }
        let mut spec: Option<(rsim_sem::ScopeId, usize)> = None;
        if let Some(v) = db.config_specs.get(&region) {
            let mut it = v.iter().enumerate();
            spec = pick(&mut it).map(|i| (region, i));
        }
        let cc_of = |p: &CfgPath| -> Option<&rsim_sem::hir::ComponentConfig> {
            let (last, init) = p.steps.split_last()?;
            let b = block_cfg(
                db,
                &CfgPath {
                    root: p.root.clone(),
                    steps: init.to_vec(),
                },
            )?;
            match b.items.get(*last)? {
                ConfigItem::Component(c) => Some(c),
                _ => None,
            }
        };
        let decl_cc = found.as_ref().and_then(cc_of);
        let spec_cc = spec.and_then(|(r, i)| db.config_specs.get(&r)?.get(i));
        let with_aspect = |c: &rsim_sem::hir::ComponentConfig| {
            c.binding.as_ref().is_some_and(|b| b.aspect.is_some())
        };
        // The binding indication: the declaration's, if it names an entity, else the spec's.
        let (bind, block) = match (decl_cc, spec_cc) {
            (Some(d), _) if with_aspect(d) => (d.binding.as_ref(), found.clone()),
            (Some(d), Some(s)) => {
                if d.binding
                    .as_ref()
                    .is_some_and(|b| !b.generic_map.is_empty() || !b.port_map.is_empty())
                {
                    return Err(unsupported(d.span, "incremental binding indications", "M3"));
                }
                (s.binding.as_ref(), found.clone())
            }
            (Some(d), None) => {
                if d.binding.is_some() {
                    return Err(unsupported(d.span, "incremental binding indications", "M3"));
                }
                (None, found.clone())
            }
            (None, Some(s)) => (
                s.binding.as_ref(),
                s.block.as_ref().map(|_| CfgPath {
                    root: CfgRoot::Spec(spec.expect("spec found").0, spec.expect("spec found").1),
                    steps: Vec::new(),
                }),
            ),
            (None, None) => (None, None),
        };
        let nested = |p: Option<CfgPath>| p.filter(|p| block_cfg(db, p).is_some());
        if let Some(b) = bind {
            let (entity, arch, cfgu) = match &b.aspect {
                Some(EntityAspect::Entity { unit, arch }) => (*unit, arch.map(|a| a.key), None),
                Some(EntityAspect::Configuration(cu)) => match find_top_of_config(db, *cu) {
                    Ok((e, a)) => (e, Some(db.units[a].name.key), Some(*cu)),
                    Err(()) => return Err(Box::new(ice(span, "configuration target"))),
                },
                Some(EntityAspect::Open) => return Ok(None),
                None => {
                    return Err(unsupported(
                        span,
                        "binding indications without an entity aspect",
                        "M3",
                    ));
                }
            };
            let (ng, np) = counts(db, entity);
            let cfgp = match cfgu {
                Some(cu) => Some(CfgPath {
                    root: CfgRoot::Unit(cu),
                    steps: Vec::new(),
                }),
                None => nested(block),
            };
            return Ok(Some(Bound {
                entity,
                arch,
                gmap: assocs(&b.generic_map, ng),
                pmap: assocs(&b.port_map, np),
                cfg: cfgp,
            }));
        }
        // Default binding (7.3.3): an entity of the component's name; maps by name.
        let block = nested(block);
        let Some((entity, arch)) = self.db.default_binding(region, comp) else {
            return Ok(None);
        };
        let db: &Db = self.db;
        let DeclKind::Component(c) = &db.decls[comp].kind else {
            return Err(Box::new(ice(span, "component")));
        };
        let UnitHir::Entity {
            generics, ports, ..
        } = &db.units[entity].hir
        else {
            return Err(Box::new(ice(span, "default-bound entity")));
        };
        let by_name = |formals: &[DeclId], locals: &[DeclId], open: A| -> Vec<A> {
            formals
                .iter()
                .map(|&f| {
                    let k = db.decls[f].name.key();
                    match locals.iter().find(|&&l| db.decls[l].name.key() == k) {
                        Some(&l) => A::Local(l),
                        None if matches!(open, A::Open) => A::Open,
                        None => A::Default,
                    }
                })
                .collect()
        };
        Ok(Some(Bound {
            entity,
            arch,
            gmap: by_name(generics, &c.generics, A::Default),
            pmap: by_name(ports, &c.ports, A::Open),
            cfg: block,
        }))
    }

    #[allow(clippy::too_many_arguments)]
    fn component(
        &mut self,
        s: ScopeId,
        region: rsim_sem::ScopeId,
        st: StmtId,
        label: Ident,
        comp: DeclId,
        cgens: &[DeclId],
        cports: &[DeclId],
        g: Vec<A>,
        p: Vec<A>,
        cfg: Option<&CfgPath>,
        depth: u32,
        span: Span,
    ) -> R<()> {
        let c = self
            .m
            .hier
            .add(Some(s), ScopeKind::Component { comp }, label);
        for (i, &cg) in cgens.iter().enumerate() {
            self.generic(c, cg, &g[i], Some(s), &[], span)?;
        }
        for (i, &cp) in cports.iter().enumerate() {
            self.port(c, cp, &p[i], Some(s), span)?;
        }
        match self.binding(region, st, comp, cfg, span)? {
            Some(b) => {
                let a = self.load_arch(b.entity, b.arch, span)?;
                self.instance(
                    Some(c),
                    label,
                    b.entity,
                    a,
                    b.gmap,
                    Some(c),
                    b.pmap,
                    Some(c),
                    b.cfg,
                    &[],
                    depth + 1,
                    span,
                )?;
            }
            None => self.warnings.push(
                Diagnostic::warning(
                    codes::UNBOUND,
                    format!(
                        "component instance `{}` is not bound to an entity",
                        label.text
                    ),
                )
                .with_primary(span, "left unbound (open)")
                .with_note(format!(
                    "no entity `{}` is visible and none is configured (LRM 7.3.3)",
                    self.db.decls[comp].name
                )),
            ),
        }
        Ok(())
    }

    fn process(
        &mut self,
        s: ScopeId,
        st: StmtId,
        name: Ident,
        postponed: bool,
        lsps: &[Lsp],
    ) -> R<()> {
        let ps = self
            .m
            .hier
            .add(Some(s), ScopeKind::Process { stmt: st }, name);
        let idx = ProcIdx(self.procs.len() as u32);
        // Merge the driven ranges per leaf: a process has one driver per scalar subelement.
        let mut per_lsp = Vec::with_capacity(lsps.len());
        let mut by_leaf: Vec<(LeafId, Range)> = Vec::new();
        for l in lsps {
            let (o, sels) = self.m.lsp_ref(self.db, l, s)?;
            let (rs, _) = self.m.select(o, &sels, l.span)?;
            by_leaf.extend(rs.iter().copied());
            per_lsp.push(rs);
        }
        by_leaf.sort_by_key(|(l, r)| (*l, r.offset));
        let mut merged: Vec<(LeafId, Range)> = Vec::new();
        for (l, r) in by_leaf {
            match merged.last_mut() {
                Some((ml, mr)) if *ml == l && r.offset <= mr.end() => {
                    let end = mr.end().max(r.end());
                    mr.len = end - mr.offset;
                }
                _ => merged.push((l, r)),
            }
        }
        let base = self.m.nets.drivers.len();
        let drivers: Vec<usize> = (base..base + merged.len()).collect();
        for &(leaf, range) in &merged {
            self.m.nets.drivers.push(DriverDecl {
                proc: idx,
                leaf,
                range,
            });
        }
        let lsps = per_lsp
            .into_iter()
            .map(|rs| {
                rs.into_iter()
                    .map(|(l, r)| {
                        let k = merged
                            .iter()
                            .position(|(ml, mr)| *ml == l && mr.contains(r))
                            .expect("merged ranges cover every lsp range");
                        (base + k, l, r)
                    })
                    .collect()
            })
            .collect();
        self.procs.push(ProcInst {
            scope: ps,
            stmt: st,
            postponed,
            drivers,
            lsps,
        });
        Ok(())
    }

    /// LRM 14.7.2: an unresolved signal has at most one source per element (process drivers
    /// and `out`/`inout`/`buffer` ports).
    fn check_sources(&mut self) {
        let mut src: HashMap<LeafId, Vec<(Range, String)>> = HashMap::new();
        for d in &self.m.nets.drivers {
            let p = &self.procs[d.proc.index()];
            src.entry(d.leaf).or_default().push((
                d.range,
                format!("process {}", self.m.hier.path_name(p.scope, Case::Original)),
            ));
        }
        for e in &self.m.nets.ports {
            if e.mode != PortMode::In {
                let f = self.m.nets.obj(self.m.nets.leaf(e.formal.0).obj);
                src.entry(e.actual.0).or_default().push((
                    e.actual.1,
                    format!(
                        "port {}{}",
                        self.m.hier.path_name(f.scope, Case::Original),
                        f.name.text
                    ),
                ));
            }
        }
        let mut leaves: Vec<_> = src.into_iter().collect();
        leaves.sort_by_key(|(l, _)| *l);
        for (l, v) in leaves {
            let leaf = self.m.nets.leaf(l);
            if leaf.resolution.is_some() {
                continue;
            }
            for i in 0..v.len() {
                for j in i + 1..v.len() {
                    if v[i].0.overlaps(v[j].0) {
                        let o = self.m.nets.obj(leaf.obj);
                        let span = self.db.decls[o.decl].span;
                        self.errors.push(
                            Diagnostic::error(
                                codes::MULTIPLE_SOURCES,
                                format!(
                                    "unresolved signal `{}{}` has several sources",
                                    self.m.hier.path_name(o.scope, Case::Original),
                                    o.name.text
                                ),
                            )
                            .with_primary(span, "declared here with an unresolved subtype")
                            .with_note(format!("source: {}", v[i].1))
                            .with_note(format!("source: {}", v[j].1))
                            .with_help("drive it from one process, or use a resolved subtype such as `std_logic`"),
                        );
                        return;
                    }
                }
            }
        }
    }
}

fn counts(db: &Db, entity: UnitId) -> (usize, usize) {
    match &db.units[entity].hir {
        UnitHir::Entity {
            generics, ports, ..
        } => (generics.len(), ports.len()),
        _ => (0, 0),
    }
}

/// (entity, architecture) a configuration configures.
fn find_top_of_config(db: &Db, cu: UnitId) -> Result<(UnitId, UnitId), ()> {
    match &db.units[cu].hir {
        UnitHir::Configuration(BlockConfig {
            target: BlockTarget::Architecture(a),
            ..
        }) => match &db.units[*a].hir {
            UnitHir::Architecture { entity, .. } => Ok((*entity, *a)),
            _ => Err(()),
        },
        _ => Err(()),
    }
}

/// `'image` of a generate parameter value, as used in path names (LRM 16.2.5).
fn image(db: &Db, param: DeclId, v: &Val) -> String {
    use rsim_sem::decl::Designator;
    let DeclKind::Object(o) = &db.decls[param].kind else {
        return String::new();
    };
    match (&db.types[db.base(o.subtype)].kind, v) {
        (rsim_sem::types::TypeKind::Enum { lits }, Val::Enum(p)) => {
            match lits.get(*p as usize).map(|&l| db.decls[l].name) {
                Some(Designator::Ident(i)) => i.key.to_string(),
                Some(Designator::Char(c)) => format!("'{c}'"),
                _ => p.to_string(),
            }
        }
        (_, v) => v.as_int().map_or_else(String::new, |x| x.to_string()),
    }
}

impl Model {
    fn m_scalar(
        &mut self,
        db: &Db,
        st: rsim_sem::SubtypeId,
        s: ScopeId,
        span: Span,
    ) -> R<crate::Bounds> {
        let r = db.scalar_range(st);
        match r {
            Some(r) => self.range(db, &r, Some(s), span),
            None => match &db.types[db.base(st)].kind {
                rsim_sem::types::TypeKind::Enum { lits } => Ok(crate::Bounds {
                    left: 0,
                    dir: rsim_sem::hir::Direction::To,
                    right: lits.len() as i64 - 1,
                }),
                _ => Ok(crate::Bounds {
                    left: i64::MIN,
                    dir: rsim_sem::hir::Direction::To,
                    right: i64::MAX,
                }),
            },
        }
    }

    /// Create a signal object in `s` with its leaves.
    #[allow(clippy::too_many_arguments)]
    fn add_obj(
        &mut self,
        db: &Db,
        s: ScopeId,
        decl: DeclId,
        kind: ObjKind,
        shape: Shape,
        init: Option<&Val>,
        at: Span,
    ) -> R<ObjId> {
        let vals = leaf_values(db, &shape, init).map_err(|m| {
            Box::new(
                Diagnostic::error(
                    codes::EVAL_FAILED,
                    format!(
                        "initial value of `{}` does not fit: {m}",
                        db.decls[decl].name
                    ),
                )
                .with_primary(at, "here"),
            )
        })?;
        let id = ObjId(self.nets.objs.len() as u32);
        let first = LeafId(self.nets.leaves.len() as u32);
        for ((path, leaf, n), init) in shape.leaves().into_iter().zip(vals) {
            let Shape::Scalar { elem, res, .. } = leaf else {
                continue;
            };
            let len = u32::try_from(n).map_err(|_| {
                Box::new(
                    Diagnostic::error(
                        codes::EVAL_FAILED,
                        format!(
                            "signal `{}` has more than 2**32 elements",
                            db.decls[decl].name
                        ),
                    )
                    .with_primary(at, "here"),
                )
            })?;
            let resolution = match res {
                None => None,
                Some(f)
                    if *elem == Elem::Logic
                        && db.decls[*f].name.key().as_str() == "resolved"
                        && crate::eval::is_std_ulogic(db, leaf.ty()) =>
                {
                    Some(LeafRes::StdLogic)
                }
                Some(f) => Some(LeafRes::Func(*f)),
            };
            self.nets.leaves.push(Leaf {
                obj: id,
                path,
                ty: rsim_kernel::SignalType { elem: *elem, len },
                resolution,
                init,
            });
        }
        let end = LeafId(self.nets.leaves.len() as u32);
        self.nets.objs.push(SigObj {
            scope: s,
            decl,
            name: match db.decls[decl].name {
                rsim_sem::decl::Designator::Ident(i) => i,
                _ => return Err(Box::new(ice(at, "signal without identifier"))),
            },
            kind,
            shape,
            leaves: (first, end),
        });
        self.hier.scopes[s.index()].objs.push(id);
        self.sigs.insert((s, decl), id);
        Ok(id)
    }

    /// The signal object and selectors a static signal name denotes, or `None` if `e`
    /// is not a signal name.
    fn name_ref(&mut self, db: &Db, e: ExprId, scope: ScopeId) -> R<Option<(ObjId, Vec<Sel>)>> {
        let ex = &db.exprs[e];
        let (prefix, sel) = match &ex.kind {
            ExprKind::Obj(d) => return Ok(self.signal(scope, *d).map(|o| (o, Vec::new()))),
            ExprKind::Index { prefix, indices } => {
                let mut v = Vec::with_capacity(indices.len());
                for &i in indices.iter() {
                    v.push(self.eval(db, i, Some(scope))?.as_int().unwrap_or(0));
                }
                (*prefix, Sel::Index(v))
            }
            ExprKind::Slice { prefix, range } => (
                *prefix,
                Sel::Slice(self.range(db, range, Some(scope), ex.span)?),
            ),
            ExprKind::Field { prefix, elem } => (*prefix, Sel::Field(*elem)),
            _ => return Ok(None),
        };
        Ok(self.name_ref(db, prefix, scope)?.map(|(o, mut v)| {
            v.push(sel);
            (o, v)
        }))
    }

    fn lsp_ref(&mut self, db: &Db, l: &Lsp, scope: ScopeId) -> R<(ObjId, Vec<Sel>)> {
        let ObjRoot::Decl(d) = l.root else {
            return Err(unsupported(
                l.span,
                "external names and implicit signals as targets",
                "M3",
            ));
        };
        let Some(o) = self.signal(scope, d) else {
            return Err(unsupported(l.span, "signals declared in packages", "M3"));
        };
        let sels = self.steps(db, &l.path, scope, l.span)?;
        Ok((o, sels))
    }

    fn steps(&mut self, db: &Db, path: &[LspStep], scope: ScopeId, span: Span) -> R<Vec<Sel>> {
        let mut v = Vec::with_capacity(path.len());
        for st in path {
            v.push(match st {
                LspStep::Field(f) => Sel::Field(*f),
                LspStep::Index(ix) => {
                    let mut xs = Vec::with_capacity(ix.len());
                    for &i in ix.iter() {
                        let x = self.eval(db, i, Some(scope))?;
                        xs.push(
                            x.as_int()
                                .ok_or_else(|| Box::new(ice(span, "non-discrete index")))?,
                        );
                    }
                    Sel::Index(xs)
                }
                LspStep::Slice(r) => Sel::Slice(self.range(db, r, Some(scope), span)?),
            });
        }
        Ok(v)
    }

    fn select(&self, o: ObjId, sels: &[Sel], span: Span) -> R<(Vec<(LeafId, Range)>, Shape)> {
        self.nets.select(o, sels).map_err(|m| {
            Box::new(
                Diagnostic::error(codes::BAD_NAME, format!("invalid static name: {m}"))
                    .with_primary(span, "this name"),
            )
        })
    }

    fn edges(
        &mut self,
        fl: &[(LeafId, Range)],
        al: &[(LeafId, Range)],
        mode: PortMode,
        span: Span,
    ) -> R<()> {
        let ok = fl.len() == al.len()
            && fl.iter().zip(al).all(|(f, a)| {
                f.1.len == a.1.len && self.nets.leaf(f.0).ty.elem == self.nets.leaf(a.0).ty.elem
            });
        if !ok {
            let n = |l: &[(LeafId, Range)]| l.iter().map(|x| u64::from(x.1.len)).sum::<u64>();
            return Err(Box::new(
                Diagnostic::error(
                    codes::PORT_MISMATCH,
                    format!("the actual has {} elements, the formal {}", n(al), n(fl)),
                )
                .with_primary(span, "associated here"),
            ));
        }
        for (f, a) in fl.iter().zip(al) {
            if f.1.len == 0 {
                continue;
            }
            self.nets.ports.push(PortEdge {
                formal: *f,
                actual: *a,
                mode,
            });
        }
        Ok(())
    }
}
