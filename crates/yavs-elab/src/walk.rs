//! The elaborator: instances, generics, generates, blocks, binding, ports and processes
//! (ir.md 4, steps 1 to 10), depth-first in statement order.

use std::collections::HashMap;

use yavs_diag::{Diagnostic, Span};
use yavs_kernel::{Elem, GuardKind, PortMode, Range};
use yavs_sem::decl::{DeclKind, ObjClass, ObjRole};
use yavs_sem::hir::{
    Actual, Arg, BlockConfig, BlockTarget, Choice, ConfigItem, EntityAspect, ExprKind, GenSelect,
    InstTarget, InstanceSel, Lsp, LspStep, Mode, ObjRoot, StmtKind, UnitHir,
};
use yavs_sem::loader::UnitKind;
use yavs_sem::{Db, DeclId, ExprId, StmtId, UnitId};
use yavs_syntax::Symbol;
use yavs_syntax::ast::Ident;

use crate::eval::{Model, Val, leaf_values, parse_override};
use crate::hier::{
    Case, DriverDecl, ImplicitKind, Leaf, LeafId, LeafRes, ObjId, ObjKind, PortEdge, ProcIdx,
    ScopeId, ScopeKind, Sel, Shape, SigObj,
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
    Conv {
        formal_conv: Option<DeclId>,
        actual_conv: Option<DeclId>,
        actual: ExprId,
    },
}

/// The associations of a map as `(formal index, association)`.
type Pairs = Vec<(usize, A)>;

fn pairs(args: &[Arg]) -> Pairs {
    let n = args
        .iter()
        .map(|a| a.formal as usize + 1)
        .max()
        .unwrap_or(0);
    assocs(args, n)
        .into_iter()
        .enumerate()
        .filter(|(i, _)| args.iter().any(|a| a.formal as usize == *i))
        .collect()
}

fn assocs(args: &[Arg], n: usize) -> Vec<A> {
    let mut v: Vec<A> = (0..n).map(|_| A::Default).collect();
    for a in args {
        let x = match &a.actual {
            Actual::Expr(e) => A::Expr(*e),
            Actual::Default => A::Default,
            Actual::Open => A::Open,
            Actual::Individual(p) => A::Parts(p.iter().map(|(s, e)| (s.to_vec(), *e)).collect()),
            // Never read: `generic` reports generic types and subprograms unsupported first.
            Actual::Subtype(_) | Actual::Subprogram(_) => A::Default,
            Actual::Converted {
                formal_conv,
                actual_conv,
                actual,
            } => A::Conv {
                formal_conv: *formal_conv,
                actual_conv: *actual_conv,
                actual: *actual,
            },
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
    Spec(yavs_sem::ScopeId, usize),
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
                Some(a) => Symbol::intern(&yavs_lib::fold(a)),
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
        .with_note("this is a bug in yavs")
        .with_help("please report it with the source that triggers it")
}

fn unsupported(span: Span, what: &str, when: &str) -> Box<Diagnostic> {
    Box::new(
        Diagnostic::error(
            codes::NOT_IMPLEMENTED,
            format!("{what} is not implemented yet in yavs's elaboration"),
        )
        .with_primary(span, "not supported yet")
        .with_note(format!(
            "planned for {when}; this is a limitation of yavs, not an error in the design"
        )),
    )
}

/// Load the body of every loaded package (not std, whose subprograms are native), so
/// elaboration-time calls and lowering see subprogram bodies. A package that needs a body
/// (a subprogram without one, a deferred constant) but has none is an error (LRM 4.8).
// ponytail: every loaded package, not a call graph walk; enough until designs with many
// unused packages get slow.
fn load_bodies(db: &mut Db) -> Result<(), Vec<Diagnostic>> {
    use yavs_sem::decl::Body;
    use yavs_sem::loader::{UnitKey, UnitSlot};
    let pkgs: Vec<UnitId> = db
        .units
        .iter()
        .filter(|(_, u)| matches!(u.hir, UnitHir::Package { .. }))
        .filter(|(_, u)| db.libs[u.lib].name.key.as_str() != "std")
        .map(|(id, _)| id)
        .collect();
    let mut errs = Vec::new();
    for p in pkgs {
        let (lib, key, name) = (db.units[p].lib, db.units[p].name.key, db.units[p].name);
        let body = UnitKey::body(key);
        let has = matches!(
            db.libs[lib].units.get(&body),
            Some(UnitSlot::Done(_) | UnitSlot::InProgress(_))
        ) || db.locate(lib, body).is_some();
        if has {
            let (l, n) = (db.libs[lib].name.text.to_string(), name.text.to_string());
            if db.load_body(&l, &n).is_err() {
                return Err(db.take_diagnostics());
            }
            continue;
        }
        let scope = db.units[p].scope;
        let need = db.scopes[scope].order.iter().copied().find(|&d| {
            let decl = &db.decls[d];
            !decl.implicit
                && match &decl.kind {
                    DeclKind::Subprogram(s) => matches!(s.body, Body::Pending),
                    DeclKind::Object(o) => o.role == ObjRole::Deferred,
                    _ => false,
                }
        });
        if let Some(d) = need {
            let what = match db.decls[d].kind {
                DeclKind::Object(_) => "deferred constant",
                _ => "subprogram",
            };
            errs.push(
                Diagnostic::error(
                    codes::BODY_MISSING,
                    format!("package `{}` needs a body, and none is analyzed", name.text),
                )
                .with_primary(
                    db.decls[d].span,
                    format!("this {what} is completed in the package body"),
                )
                .with_secondary(name.span, "package declared here")
                .with_help(format!(
                    "analyze the file with `package body {} is ... end;` into library `{}`",
                    name.text, db.libs[lib].name.text
                )),
            );
        }
    }
    if !errs.is_empty() {
        return Err(errs);
    }
    // Bodies of generic package and subprogram instances (cloned from the generic body).
    let insts: Vec<yavs_sem::InstId> = db.insts.iter().map(|(i, _)| i).collect();
    for i in insts {
        if db.instance_body(i).is_err() {
            return Err(db.take_diagnostics());
        }
    }
    Ok(())
}

pub(crate) fn elaborate(
    db: &mut Db,
    top: Top,
    overrides: &[(String, String)],
    cfg: &ElabConfig,
) -> Result<Elaborated, Vec<Diagnostic>> {
    load_bodies(db)?;
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
    // Every signal of a loaded package exists, read or not (#183); driven ones already do.
    if e.errors.is_empty() {
        let sigs: Vec<(DeclId, Span)> = e
            .db
            .units
            .iter()
            .filter(|(_, u)| matches!(u.hir, UnitHir::Package { .. }))
            .flat_map(|(_, u)| e.db.scopes[u.scope].order.iter().copied())
            .filter(|&d| {
                matches!(&e.db.decls[d].kind, DeclKind::Object(o) if o.class == ObjClass::Signal)
            })
            .map(|d| (d, e.db.decls[d].span))
            .collect();
        for (d, at) in sigs {
            if let Err(x) = e.m.signal_or_package(e.db, ScopeId(0), d, at) {
                e.errors.push(*x);
            }
        }
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
        convs: std::mem::take(&mut e.m.convs),
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
                let key = yavs_lib::fold(name);
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
                "slice 3 (with yavs-lower)",
            ));
        };
        let key = decl.name.key().as_str().to_string();
        let ov = overrides
            .iter()
            .rev()
            .find(|(n, _)| yavs_lib::fold(n) == key);
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
            (_, A::Parts(_) | A::Conv { .. }) => {
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
                            if b.dir == yavs_sem::hir::Direction::To {
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

    /// `s'stable(t)`, `s'quiet(t)`, `s'delayed(t)`, `s'transaction` (K2): a kernel-maintained
    /// signal over the elements of the prefix.
    fn implicit_signal(&mut self, s: ScopeId, d: DeclId) -> R<()> {
        use yavs_sem::hir::{AttrPrefix, PredefAttr};
        let db: &Db = self.db;
        let decl = &db.decls[d];
        let span = decl.span;
        let DeclKind::Object(o) = &decl.kind else {
            return Err(Box::new(ice(span, "implicit signal")));
        };
        let Some(def) = o.default else {
            return Err(Box::new(ice(span, "implicit signal without definition")));
        };
        let ExprKind::Attr {
            attr,
            prefix: AttrPrefix::Expr(prefix),
            args,
        } = &db.exprs[def].kind
        else {
            return Err(Box::new(ice(span, "implicit signal definition")));
        };
        let (attr, prefix, arg) = (*attr, *prefix, args.first().copied());
        let Some((po, sels)) = self.m.name_ref(self.db, prefix, s)? else {
            return Err(unsupported(span, "implicit signals of this prefix", "M3"));
        };
        let (rs, pshape) = self.m.select(po, &sels, span)?;
        let [(leaf, range)] = rs.as_slice() else {
            return Err(unsupported(
                span,
                "implicit signals of a record (several kernel signals)",
                "M3",
            ));
        };
        let t = match arg {
            Some(a) => {
                let v = self.m.eval(self.db, a, Some(s))?;
                let fs = v.as_int().unwrap_or(0);
                if fs < 0 {
                    return Err(Box::new(
                        Diagnostic::error(
                            codes::EVAL_FAILED,
                            format!("the time of `'{}` is negative", attr.name()),
                        )
                        .with_primary(self.db.exprs[a].span, "negative"),
                    ));
                }
                yavs_kernel::Time::from_vhdl(fs).map_err(|_| {
                    Box::new(
                        Diagnostic::error(codes::EVAL_FAILED, "time out of range")
                            .with_primary(span, "here"),
                    )
                })?
            }
            None => yavs_kernel::Time::ZERO,
        };
        let kind = match attr {
            PredefAttr::Stable => ImplicitKind::Stable(t),
            PredefAttr::Quiet => ImplicitKind::Quiet(t),
            PredefAttr::Delayed => ImplicitKind::Delayed(t),
            PredefAttr::Transaction => ImplicitKind::Transaction,
            _ => return Err(Box::new(ice(span, "not an implicit signal attribute"))),
        };
        let src = (*leaf, *range);
        let db: &Db = self.db;
        let (shape, init) = match kind {
            ImplicitKind::Delayed(_) => {
                let init = self.m.nets.leaf(src.0).init.clone();
                (pshape, Some(init))
            }
            _ => {
                let st = o.subtype;
                let Some(sh) = self.m.shape(db, st, Some(s), span)? else {
                    return Err(Box::new(ice(span, "implicit signal subtype")));
                };
                (sh, None)
            }
        };
        let obj = self
            .m
            .add_obj(self.db, s, d, ObjKind::Implicit, shape, None, span)?;
        let (a, b) = self.m.nets.obj(obj).leaves;
        if b.0 != a.0 + 1 {
            return Err(Box::new(ice(span, "implicit signal of several leaves")));
        }
        let src_elem = self.m.nets.leaves[src.0.index()].ty.elem;
        let l = &mut self.m.nets.leaves[a.index()];
        // The kernel decides the representation of these signals (K2): one byte per
        // element for 'stable, 'quiet and 'transaction; the prefix's for 'delayed.
        l.ty = match kind {
            ImplicitKind::Delayed(_) => yavs_kernel::SignalType {
                elem: src_elem,
                len: src.1.len,
            },
            _ => yavs_kernel::SignalType::bytes(1, 1),
        };
        if let Some(init) = init {
            l.init = init;
        }
        l.implicit = Some((kind, src.0, src.1));
        Ok(())
    }

    /// Signal declarations of a declarative region (constants are evaluated on demand).
    fn decls(&mut self, s: ScopeId, region: yavs_sem::ScopeId) -> R<()> {
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
                        return Err(unconstrained_signal(o, span));
                    };
                    let guard = o.signal_kind.map(|k| match k {
                        yavs_syntax::ast::SignalKind::Bus => GuardKind::Bus,
                        yavs_syntax::ast::SignalKind::Register => GuardKind::Register,
                    });
                    let init = match o.default {
                        Some(e) => Some(self.m.eval(db, e, Some(s))?),
                        None => None,
                    };
                    let obj = self.m.add_obj(
                        self.db,
                        s,
                        d,
                        ObjKind::Signal,
                        shape,
                        init.as_ref(),
                        span,
                    )?;
                    if let Some(g) = guard {
                        // ponytail: disconnection specifications are not in the HIR yet, so
                        // the disconnection time is 0 ns.
                        let (a, b) = self.m.nets.obj(obj).leaves;
                        for l in &mut self.m.nets.leaves[a.index()..b.index()] {
                            l.guard = Some(g);
                        }
                    }
                }
                (ObjClass::Signal, ObjRole::Guard) => {
                    // The implicit GUARD signal of a guarded block (LRM 11.2), driven by
                    // the block's guard process.
                    let span = decl.span;
                    let Some(shape) = self.m.shape(db, o.subtype, Some(s), span)? else {
                        return Err(Box::new(ice(span, "GUARD subtype")));
                    };
                    self.m
                        .add_obj(self.db, s, d, ObjKind::Signal, shape, None, span)?;
                }
                (ObjClass::Signal, ObjRole::ImplicitSignal) => {
                    self.implicit_signal(s, d)?;
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
        if o.class == ObjClass::Variable {
            return Err(unsupported(span, "variable ports (2019)", "M3"));
        }
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
                                "slice 4 (implicit processes, yavs-lower)",
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
            (
                A::Conv {
                    formal_conv,
                    actual_conv,
                    actual,
                },
                Some(sc),
            ) => {
                let aspan = db.exprs[*actual].span;
                let Some((ao, asels)) = self.m.name_ref(self.db, *actual, sc)? else {
                    return Err(unsupported(
                        aspan,
                        "converted expression actuals",
                        "slice 4",
                    ));
                };
                let (al, ashape) = self.m.select(ao, &asels, aspan)?;
                let Some(shape) = own else {
                    return Err(unsupported(
                        span,
                        "conversions of unconstrained ports",
                        "M3",
                    ));
                };
                let (fty, aty) = (shape.ty(), ashape.ty());
                let fo = self
                    .m
                    .add_obj(self.db, s, p, kind, shape, init.as_ref(), span)?;
                let fl = self.m.select(fo, &[], span)?.0;
                let db: &Db = self.db;
                // `actual_conv(actual)` feeds an `in`/`inout` formal, `formal_conv(formal)`
                // its actual.
                let idx = |m: &mut crate::Model, f: Option<DeclId>, from, to| -> R<Option<u32>> {
                    match f {
                        None => Ok(None),
                        Some(f) => {
                            let c = crate::eval::conv_fn(db, f, from, to, aspan)?;
                            m.convs.push(c);
                            Ok(Some(m.convs.len() as u32 - 1))
                        }
                    }
                };
                let to_formal = idx(&mut self.m, *actual_conv, aty, fty)?;
                let to_actual = idx(&mut self.m, *formal_conv, fty, aty)?;
                let ([f], [a]) = (fl.as_slice(), al.as_slice()) else {
                    return Err(unsupported(aspan, "conversions of composite ports", "M3"));
                };
                self.m.nets.ports.push(PortEdge {
                    formal: *f,
                    actual: *a,
                    mode,
                    to_formal,
                    to_actual,
                });
                return Ok(());
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
                let pscope = match &stmt.kind {
                    StmtKind::Process { scope, .. } => Some(*scope),
                    _ => None,
                };
                self.process(s, st, name, postponed, &lsps)?;
                // Implicit signals ('stable, ...) are declared in the process's region.
                match pscope {
                    Some(r) => {
                        let ps = self.procs.last().expect("just added").scope;
                        self.decls(ps, r)
                    }
                    None => Ok(()),
                }
            }
            StmtKind::Block {
                scope: bscope,
                guard,
                generics,
                generic_map,
                ports,
                port_map,
                body,
            } => {
                let (bscope, body, guard) = (*bscope, body.to_vec(), *guard);
                let (gens, pts) = (generics.to_vec(), ports.to_vec());
                let (ga, pa) = (assocs(generic_map, gens.len()), assocs(port_map, pts.len()));
                let b = self.m.hier.add(Some(s), ScopeKind::Block, named(db)?);
                let bcfg = cfg.and_then(|c| self.sub_cfg(c, st, None));
                // Block generics and ports (LRM 11.2): like an instance's, actuals in the
                // enclosing region.
                for (g, a) in gens.iter().zip(&ga) {
                    self.generic(b, *g, a, Some(s), &[], span)?;
                }
                for (p, a) in pts.iter().zip(&pa) {
                    self.port(b, *p, a, Some(s), span)?;
                }
                self.decls(b, bscope)?;
                if let Some(g) = guard {
                    if self.m.signal(b, g).is_none() {
                        let gspan = self.db.decls[g].span;
                        let DeclKind::Object(o) = &self.db.decls[g].kind else {
                            return Err(Box::new(ice(span, "GUARD is not an object")));
                        };
                        let Some(shape) = self.m.shape(self.db, o.subtype, Some(b), gspan)? else {
                            return Err(Box::new(ice(span, "GUARD subtype")));
                        };
                        self.m
                            .add_obj(self.db, b, g, ObjKind::Signal, shape, None, gspan)?;
                    }
                    // The guard process `GUARD <= guard_expression` (LRM 11.2): its
                    // statement is the block statement; it drives GUARD.
                    let o = self.m.signal(b, g).expect("GUARD just elaborated");
                    let n = Symbol::intern("_guard");
                    let name = Ident {
                        text: n,
                        key: n,
                        span,
                    };
                    self.process_refs(b, st, name, false, vec![(o, Vec::new(), span)])?;
                }
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
                        yavs_sem::hir::Direction::To => b.left + i as i64,
                        yavs_sem::hir::Direction::Downto => b.left - i as i64,
                    };
                    let db: &Db = self.db;
                    let val = match db.types[db.base(match &db.decls[param].kind {
                        DeclKind::Object(o) => o.subtype,
                        _ => return Err(Box::new(ice(span, "generate parameter"))),
                    })]
                    .kind
                    {
                        yavs_sem::types::TypeKind::Enum { .. } => Val::Enum(v as u32),
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
        gscope: yavs_sem::ScopeId,
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
        region: yavs_sem::ScopeId,
        st: StmtId,
        comp: DeclId,
        cfg: Option<&CfgPath>,
        span: Span,
    ) -> R<Option<Bound>> {
        let db: &Db = self.db;
        let pick = |items: &mut dyn Iterator<Item = (usize, &yavs_sem::hir::ComponentConfig)>| {
            let items: Vec<_> = items.filter(|(_, c)| c.component == comp).collect();
            let named = |c: &yavs_sem::hir::ComponentConfig| matches!(&c.instances, InstanceSel::Labels(l) if l.contains(&st));
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
        let mut spec: Option<(yavs_sem::ScopeId, usize)> = None;
        if let Some(v) = db.config_specs.get(&region) {
            let mut it = v.iter().enumerate();
            spec = pick(&mut it).map(|i| (region, i));
        }
        let cc_of = |p: &CfgPath| -> Option<&yavs_sem::hir::ComponentConfig> {
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
        let with_aspect = |c: &yavs_sem::hir::ComponentConfig| {
            c.binding.as_ref().is_some_and(|b| b.aspect.is_some())
        };
        // The primary binding indication: the declaration's, if it names an entity, else the
        // specification's; one without an entity aspect is incremental (LRM 7.3.2.3) over
        // the specification's or the default binding.
        let (bind, block) = match (decl_cc, spec_cc) {
            (Some(d), _) if with_aspect(d) => (d.binding.as_ref(), found.clone()),
            (Some(_), Some(s)) => (s.binding.as_ref(), found.clone()),
            (Some(_), None) => (None, found.clone()),
            (None, Some(s)) => (
                s.binding.as_ref(),
                s.block.as_ref().map(|_| CfgPath {
                    root: CfgRoot::Spec(spec.expect("spec found").0, spec.expect("spec found").1),
                    steps: Vec::new(),
                }),
            ),
            (None, None) => (None, None),
        };
        let mut incremental: Vec<(Pairs, Pairs)> = Vec::new();
        let owned = |b: &yavs_sem::hir::Binding| (pairs(&b.generic_map), pairs(&b.port_map));
        if let Some(b) = bind.filter(|b| b.aspect.is_none()) {
            incremental.push(owned(b));
        }
        if let Some(b) = decl_cc
            .filter(|d| !with_aspect(d))
            .and_then(|d| d.binding.as_ref())
            .filter(|b| bind.is_none_or(|x| !std::ptr::eq(*b, x)))
        {
            incremental.push(owned(b));
        }
        let nested = |p: Option<CfgPath>| p.filter(|p| block_cfg(db, p).is_some());
        let mut bound = match bind.filter(|b| b.aspect.is_some()) {
            Some(b) => {
                let (entity, arch, cfgu) = match &b.aspect {
                    Some(EntityAspect::Entity { unit, arch }) => (*unit, arch.map(|a| a.key), None),
                    Some(EntityAspect::Configuration(cu)) => match find_top_of_config(db, *cu) {
                        Ok((e, a)) => (e, Some(db.units[a].name.key), Some(*cu)),
                        Err(()) => return Err(Box::new(ice(span, "configuration target"))),
                    },
                    _ => return Ok(None),
                };
                let (ng, np) = counts(db, entity);
                let cfgp = match cfgu {
                    Some(cu) => Some(CfgPath {
                        root: CfgRoot::Unit(cu),
                        steps: Vec::new(),
                    }),
                    None => nested(block),
                };
                Bound {
                    entity,
                    arch,
                    gmap: assocs(&b.generic_map, ng),
                    pmap: assocs(&b.port_map, np),
                    cfg: cfgp,
                }
            }
            None => {
                let block = nested(block);
                match self.default_bound(region, comp, span)? {
                    Some(mut b) => {
                        b.cfg = block;
                        b
                    }
                    None => return Ok(None),
                }
            }
        };
        // Incremental binding: its associations replace the primary's (NVC's permissive
        // reading; the LRM allows only formals the primary left open).
        for (g, p) in incremental {
            for (dst, src) in [(&mut bound.gmap, g), (&mut bound.pmap, p)] {
                for (i, a) in src {
                    if let Some(d) = dst.get_mut(i)
                        && !matches!(a, A::Default)
                    {
                        *d = a;
                    }
                }
            }
        }
        Ok(Some(bound))
    }

    /// Default binding (LRM 7.3.3): an entity of the component's name, maps by name.
    fn default_bound(
        &mut self,
        region: yavs_sem::ScopeId,
        comp: DeclId,
        span: Span,
    ) -> R<Option<Bound>> {
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
            cfg: None,
        }))
    }

    #[allow(clippy::too_many_arguments)]
    fn component(
        &mut self,
        s: ScopeId,
        region: yavs_sem::ScopeId,
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
        let mut refs = Vec::with_capacity(lsps.len());
        for l in lsps {
            let (o, sels) = self.m.lsp_ref(self.db, l, s)?;
            refs.push((o, sels, l.span));
        }
        self.process_refs(s, st, name, postponed, refs)
    }

    /// A process instance driving the given static names (one entry per driver slot).
    fn process_refs(
        &mut self,
        s: ScopeId,
        st: StmtId,
        name: Ident,
        postponed: bool,
        refs: Vec<(ObjId, Vec<Sel>, Span)>,
    ) -> R<()> {
        let ps = self
            .m
            .hier
            .add(Some(s), ScopeKind::Process { stmt: st }, name);
        let idx = ProcIdx(self.procs.len() as u32);
        // Merge the driven ranges per leaf: a process has one driver per scalar subelement.
        let mut per_lsp = Vec::with_capacity(refs.len());
        let mut by_leaf: Vec<(LeafId, Range)> = Vec::new();
        for (o, sels, span) in refs {
            let (rs, _) = self.m.select(o, &sels, span)?;
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
    use yavs_sem::decl::Designator;
    let DeclKind::Object(o) = &db.decls[param].kind else {
        return String::new();
    };
    match (&db.types[db.base(o.subtype)].kind, v) {
        (yavs_sem::types::TypeKind::Enum { lits }, Val::Enum(p)) => {
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
        st: yavs_sem::SubtypeId,
        s: ScopeId,
        span: Span,
    ) -> R<crate::Bounds> {
        let r = db.scalar_range(st);
        match r {
            Some(r) => self.range(db, &r, Some(s), span),
            None => match &db.types[db.base(st)].kind {
                yavs_sem::types::TypeKind::Enum { lits } => Ok(crate::Bounds {
                    left: 0,
                    dir: yavs_sem::hir::Direction::To,
                    right: lits.len() as i64 - 1,
                }),
                _ => Ok(crate::Bounds {
                    left: i64::MIN,
                    dir: yavs_sem::hir::Direction::To,
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
                ty: yavs_kernel::SignalType { elem: *elem, len },
                resolution,
                guard: None,
                implicit: None,
                init,
            });
        }
        let end = LeafId(self.nets.leaves.len() as u32);
        self.nets.objs.push(SigObj {
            scope: s,
            decl,
            name: match db.decls[decl].name {
                yavs_sem::decl::Designator::Ident(i) => i,
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
            ExprKind::Obj(d) => {
                let o = self.signal_or_package(db, scope, *d, ex.span)?;
                return Ok(o.map(|o| (o, Vec::new())));
            }
            ExprKind::External(x) => {
                return Ok(Some((self.external(db, *x, scope)?, Vec::new())));
            }
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

    /// Bind external signal name `x` (LRM 8.7) against the hierarchy elaborated so far
    /// (GHDL #548: a target that is not elaborated yet is an error, never a guess).
    pub(crate) fn external(
        &mut self,
        db: &Db,
        x: yavs_sem::ExternalId,
        scope: ScopeId,
    ) -> R<ObjId> {
        use yavs_syntax::ast::{ExternalClass, PathKind};
        let ext = &db.externals[x];
        let span = ext.span;
        let path: Vec<String> = ext.path.iter().map(|(i, _)| i.text.to_string()).collect();
        let shown = path.join(".");
        let fail = |why: String| {
            Box::new(
                Diagnostic::error(
                    codes::EXTERNAL_NOT_FOUND,
                    format!("external name `{shown}` cannot be bound: {why}"),
                )
                .with_primary(span, "external name")
                .with_note(
                    "external names are bound in elaboration order: the target must be \
                     elaborated before the external name (LRM 14.4.1)",
                ),
            )
        };
        if ext.class != ExternalClass::Signal {
            return Err(unsupported(span, "external constants and variables", "M3"));
        }
        let Some(((last, _), steps)) = ext.path.split_last() else {
            return Err(fail("empty path".into()));
        };
        if ext.path_kind == PathKind::Package {
            // `@lib.pkg.obj`: the signal declared in the package.
            let [(l, _), (p, _)] = steps else {
                return Err(unsupported(span, "nested package paths", "M3"));
            };
            let unit = db
                .libs
                .iter()
                .find(|(_, lib)| lib.name.key == l.key)
                .and_then(|(_, lib)| {
                    match lib.units.get(&yavs_sem::loader::UnitKey::Primary(p.key)) {
                        Some(yavs_sem::loader::UnitSlot::Done(u)) => Some(*u),
                        _ => None,
                    }
                });
            let Some(u) = unit else {
                return Err(fail(format!(
                    "package `{}.{}` is not analyzed",
                    l.text, p.text
                )));
            };
            let d = db.scopes[db.units[u].scope]
                .names
                .get(&last.key)
                .and_then(|v| v.first().copied());
            let Some(d) = d else {
                return Err(fail(format!(
                    "package `{}` declares no `{}`",
                    p.text, last.text
                )));
            };
            return self
                .signal_or_package(db, scope, d, span)?
                .ok_or_else(|| fail(format!("`{}` is not a signal", last.text)));
        }
        let (mut cur, rest) = match ext.path_kind {
            PathKind::Absolute => {
                let root = ScopeId(0);
                let Some(((first, _), rest)) = steps.split_first() else {
                    return Err(fail("an absolute path names the top level first".into()));
                };
                if self.hier.scope(root).name.key != first.key {
                    return Err(fail(format!("the top level is not `{}`", first.text)));
                }
                (root, rest)
            }
            _ => {
                // Relative: from the innermost enclosing region, `^` steps up.
                let ups = match ext.path_kind {
                    PathKind::Relative(n) => n,
                    _ => 0,
                };
                let mut c = scope;
                while matches!(self.hier.scope(c).kind, ScopeKind::Process { .. }) {
                    c = self.hier.scope(c).parent.expect("a process has a parent");
                }
                for _ in 0..ups {
                    c = match self.hier.scope(c).parent {
                        Some(p)
                            if matches!(self.hier.scope(p).kind, ScopeKind::Component { .. }) =>
                        {
                            self.hier
                                .scope(p)
                                .parent
                                .ok_or_else(|| fail("`^` above the top".into()))?
                        }
                        Some(p) => p,
                        None => return Err(fail("`^` above the top level".into())),
                    };
                }
                (c, steps)
            }
        };
        for (name, index) in rest {
            let want = match index {
                Some(i) => {
                    let v = self.eval(db, *i, Some(scope))?;
                    Some(v.as_int().map(|x| x.to_string()).unwrap_or_default())
                }
                None => None,
            };
            let child = self.hier.scope(cur).children.iter().copied().find_map(|c| {
                let sc = self.hier.scope(c);
                // A component scope is transparent: its entity instance has the label.
                let c = match sc.kind {
                    ScopeKind::Component { .. } => *sc.children.first()?,
                    _ => c,
                };
                let sc = self.hier.scope(c);
                (sc.name.key == name.key && sc.index == want).then_some(c)
            });
            cur = child.ok_or_else(|| {
                fail(format!(
                    "no elaborated region `{}` in {}",
                    name.text,
                    self.hier.path_name(cur, Case::Original)
                ))
            })?;
        }
        self.hier
            .scope(cur)
            .objs
            .iter()
            .copied()
            .find(|&o| self.nets.obj(o).name.key == last.key)
            .ok_or_else(|| {
                fail(format!(
                    "{} has no signal `{}` (yet)",
                    self.hier.path_name(cur, Case::Original),
                    last.text
                ))
            })
    }

    /// The signal object of `d` seen from `scope`; a signal declared in a package is
    /// elaborated on first use, in its package's scope.
    fn signal_or_package(
        &mut self,
        db: &Db,
        scope: ScopeId,
        d: DeclId,
        at: Span,
    ) -> R<Option<ObjId>> {
        if let Some(o) = self.signal(scope, d) {
            return Ok(Some(o));
        }
        let decl = &db.decls[d];
        let DeclKind::Object(o) = &decl.kind else {
            return Ok(None);
        };
        if o.class != ObjClass::Signal {
            return Ok(None);
        }
        let region = decl.scope;
        let Some(unit) = db.scopes[region]
            .owner
            .and_then(|ow| match db.decls[ow].kind {
                DeclKind::Unit(u) if db.units[u].kind == UnitKind::Package => Some(u),
                _ => None,
            })
        else {
            return Ok(None);
        };
        let ps = match self.pkg_scopes.get(&unit) {
            Some(&p) => p,
            None => {
                let lib = db.units[unit].lib;
                let ls = match self.lib_scopes.get(&lib) {
                    Some(&l) => l,
                    None => {
                        let l = self.hier.add(None, ScopeKind::Library, db.libs[lib].name);
                        self.lib_scopes.insert(lib, l);
                        l
                    }
                };
                let p = self
                    .hier
                    .add(Some(ls), ScopeKind::Package { unit }, db.units[unit].name);
                self.pkg_scopes.insert(unit, p);
                p
            }
        };
        if let Some(&obj) = self.sigs.get(&(ps, d)) {
            return Ok(Some(obj));
        }
        let Some(shape) = self.shape(db, o.subtype, None, decl.span)? else {
            return Err(unconstrained_signal(o, at));
        };
        if o.signal_kind.is_some() {
            return Err(unsupported(decl.span, "guarded signals in packages", "M3"));
        }
        let init = match o.default {
            Some(e) => Some(self.eval(db, e, None)?),
            None => None,
        };
        let obj = self.add_obj(db, ps, d, ObjKind::Signal, shape, init.as_ref(), decl.span)?;
        Ok(Some(obj))
    }

    fn lsp_ref(&mut self, db: &Db, l: &Lsp, scope: ScopeId) -> R<(ObjId, Vec<Sel>)> {
        let d = match l.root {
            ObjRoot::Decl(d) => d,
            ObjRoot::External(x) => {
                let o = self.external(db, x, scope)?;
                let sels = self.steps(db, &l.path, scope, l.span)?;
                return Ok((o, sels));
            }
            ObjRoot::ImplicitSignal(_) => {
                return Err(unsupported(l.span, "implicit signals as targets", "M3"));
            }
        };
        let Some(o) = self.signal_or_package(db, scope, d, l.span)? else {
            return Err(Box::new(ice(l.span, "driven signal without object")));
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
                to_formal: None,
                to_actual: None,
            });
        }
        Ok(())
    }
}

/// A signal whose subtype is not fully constrained: a 2019 signal takes its bounds from
/// its initial value (sem accepts it, #175); anything else is a bug.
fn unconstrained_signal(o: &yavs_sem::decl::Object, span: Span) -> Box<Diagnostic> {
    if o.bounds == yavs_sem::types::BoundsFrom::Initializer {
        unsupported(
            span,
            "signals that take their bounds from the initial value (2019)",
            "M3",
        )
    } else {
        Box::new(ice(span, "signal of an unconstrained subtype"))
    }
}
