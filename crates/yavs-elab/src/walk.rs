//! The elaborator: instances, generics, generates, blocks, binding, ports and processes
//! (ir.md 4, steps 1 to 10), depth-first in statement order.

use std::collections::HashMap;

use yavs_diag::{Diagnostic, Span};
use yavs_kernel::{Elem, GuardKind, PortMode, Range};
use yavs_sem::decl::{Alias, DeclKind, ObjClass, ObjRole};
use yavs_sem::external::ExtOwner;
use yavs_sem::generic::GenericActual;
use yavs_sem::hir::{
    Actual, Arg, BlockConfig, BlockTarget, Choice, ConfigItem, Delay, EntityAspect, ExprKind,
    GenSelect, InstTarget, InstanceSel, Lsp, LspStep, Mode, ObjRoot, PureRead, Rhs, Sensitivity,
    StmtKind, UnitHir, WaveElem, Waveform,
};
use yavs_sem::loader::UnitKind;
use yavs_sem::stat::Staticness;
use yavs_sem::types::{Constrainedness, TypeKind};
use yavs_sem::{Db, DeclId, ExprId, StmtId, UnitId};
use yavs_syntax::Symbol;
use yavs_syntax::ast::{ExternalClass, Ident};

use crate::eval::{Model, Val, leaf_values, parse_override};
use crate::hier::{
    Case, DriverDecl, ImplicitKind, Leaf, LeafId, LeafRes, ObjId, ObjKind, PortEdge, ProcIdx,
    ScopeId, ScopeKind, Sel, Shape, SigObj, Whole,
};
use crate::{ElabConfig, Elaborated, ProcInst, Top, VarObj, codes};

type R<T> = Result<T, Box<Diagnostic>>;

/// How one formal is associated (from a HIR map, or a default binding by name).
enum A {
    /// The expression and whether it was written `inertial`.
    Expr(ExprId, bool),
    /// A component local of the same name (default binding, LRM 7.3.3).
    Local(DeclId),
    Open,
    Default,
    /// Sub-element associations: path, actual, formal part, written `inertial`.
    Parts(Vec<yavs_sem::hir::IndividualPart>),
    /// The actual of a generic type, subprogram or package.
    Generic(GenericActual),
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
            Actual::Expr(e) => A::Expr(*e, a.inertial),
            Actual::Default => A::Default,
            Actual::Open => A::Open,
            Actual::Individual(p) => A::Parts(p.to_vec()),
            Actual::Subtype(st) => A::Generic(GenericActual::Subtype(*st)),
            Actual::Subprogram(d) => A::Generic(GenericActual::Subprogram(*d)),
            Actual::Package(i) => A::Generic(GenericActual::Package(*i)),
            Actual::Converted {
                formal_conv,
                actual_conv,
                actual,
                ..
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
    foreign_archs: Vec<crate::ForeignArch>,
    errors: Vec<Diagnostic>,
    warnings: Vec<Diagnostic>,
    cfg: ElabConfig,
    /// The statement being elaborated: the instance or block whose port map is elaborated.
    stmt: Option<StmtId>,
    /// Implicit aliases of external names used in processes, by region: bound once the
    /// whole hierarchy is elaborated (#93).
    pending: Vec<(ScopeId, DeclId)>,
    /// `idle_reads` of a process statement (the same for every instance of it): the signals
    /// read, or `None` where the process is not pure.
    idle_cache: HashMap<StmtId, Option<Vec<PureRead>>>,
    /// `idle_parts` left out a read that no condition of the instance can meet.
    idle_dead: bool,
    /// Processes whose drivers are rooted at external names: their drivers are elaborated
    /// once the whole hierarchy is (#93).
    deferred: Vec<(ProcIdx, ScopeId, Vec<Lsp>)>,
    /// Aliases of the external names used in package subprograms, by package (#255).
    pkg_uses: Vec<(UnitId, DeclId)>,
    /// How many units were loaded when the package bodies were last loaded (`load_new_bodies`).
    units_seen: usize,
    /// While `prog_source` runs: `static_signal_parts` of each expression seen so far (one scope).
    parts_memo: Option<HashMap<ExprId, SigParts>>,
}

type SigParts = Option<(Vec<(LeafId, Range)>, Shape)>;

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

/// E2103 for generic type, subprogram or package `d` without an actual.
fn no_generic_actual(db: &Db, d: DeclId, at: Span) -> Box<Diagnostic> {
    let d = &db.decls[d];
    Box::new(
        Diagnostic::error(
            codes::GENERIC_NO_VALUE,
            format!("generic `{}` has no actual", d.name),
        )
        .with_primary(at, "instantiated here without an actual for it")
        .with_secondary(d.span, "generic type, subprogram or package declared here")
        .with_help(
            "associate it in the generic map of an instance; `-g` gives only generic constants",
        ),
    )
}

fn ice(span: Span, what: &str) -> Diagnostic {
    Diagnostic::error(codes::INTERNAL, format!("internal error: {what}"))
        .with_primary(span, "while elaborating this")
        .with_note("this is a bug in yavs")
        .with_help("please report it with the source that triggers it")
}

/// An expression actual of an unconstrained port whose subtype gives no bounds (LRM 6.5.6.3:
/// it must have a globally static subtype).
fn not_static_subtype(at: Span, port: Span, decl: &yavs_sem::decl::Decl) -> Box<Diagnostic> {
    Box::new(
        Diagnostic::error(
            codes::PORT_UNCONSTRAINED,
            format!(
                "the expression associated with unconstrained port `{}` has no globally static subtype",
                decl.name
            ),
        )
        .with_primary(at, "the bounds of this expression are not known before simulation")
        .with_secondary(port, "port declared here")
        .with_note("LRM 6.5.6.3: an expression actual of an unconstrained port needs a globally static subtype")
        .with_help("qualify the expression with a constrained subtype, such as `t(7 downto 0)'(expr)`"),
    )
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

/// Whether `lib` is std or ieee, built into yavs: no package of them declares a signal or a
/// shared variable, so the scans for those need not decode their declarations (#426).
fn builtin_lib(db: &Db, lib: yavs_sem::ids::LibId) -> bool {
    matches!(db.libs[lib].source, yavs_sem::loader::LibSource::StdDir(_))
}

/// The aliases of the external names used in the subprograms of every loaded package (not
/// std), by package: they belong to the package and bind once the hierarchy is elaborated
/// (#255). Made before any elaboration-time call lowers the packages.
/// A package declaration naming the hierarchy (not another package's object) is an error:
/// packages are elaborated before the design hierarchy (LRM 14.4.1, 8.7).
fn package_externals(db: &mut Db) -> Result<Vec<(UnitId, DeclId)>, Vec<Diagnostic>> {
    // No external name in any loaded unit: nothing to find in every subprogram body.
    if db.externals.is_empty() {
        return Ok(Vec::new());
    }
    let units: Vec<UnitId> = db
        .units
        .iter()
        .filter(|(_, u)| db.libs[u.lib].name.key.as_str() != "std")
        .map(|(id, _)| id)
        .collect();
    let mut out = Vec::new();
    let mut errs = Vec::new();
    for u in units {
        for r in db.package_regions(u).unwrap_or_default() {
            for d in db.decls_with_instances(r) {
                let e = match &db.decls[d].kind {
                    DeclKind::Object(o) => o.default,
                    DeclKind::Alias(Alias::Object { target, .. }) => Some(*target),
                    _ => None,
                };
                if let Some(e) = e {
                    db.walk_expr(e, &mut |x| {
                        if let ExprKind::External(x) = db.exprs[x].kind {
                            let ext = &db.externals[x];
                            let path: Vec<String> =
                                ext.path.iter().map(|(i, _)| i.text.to_string()).collect();
                            errs.push(
                                Diagnostic::error(
                                    codes::EXTERNAL_NOT_FOUND,
                                    format!(
                                        "external name `{}` cannot be bound: a package is \
                                         elaborated before the design hierarchy",
                                        path.join(".")
                                    ),
                                )
                                .with_primary(ext.span, "external name in a package declaration")
                                .with_note(
                                    "a package declaration can name objects of other packages \
                                     (`@lib.pkg.obj`); a subprogram of the package can name the \
                                     hierarchy, bound when it is called (LRM 8.7, 14.4.1)",
                                ),
                            );
                        }
                    });
                }
                for m in db.decl_subprograms(d) {
                    let Some(sc) = db.sub_scope(m) else { continue };
                    for &h in db.alias_external_uses(ExtOwner::Sub(m), sc) {
                        if !out.contains(&(u, h)) {
                            out.push((u, h));
                        }
                    }
                }
            }
        }
    }
    if errs.is_empty() { Ok(out) } else { Err(errs) }
}

/// Load the body of every loaded package (not std, whose subprograms are native), so
/// elaboration-time calls and lowering see subprogram bodies. A package that needs a body
/// (a subprogram without one, a deferred constant) but has none is an error (LRM 4.8).
// ponytail: every loaded package, not a call graph walk; enough until designs with many
// unused packages get slow.
pub fn load_bodies(db: &mut Db) -> Result<(), Vec<Diagnostic>> {
    load_bodies_in(db, None)
}

/// [`load_bodies`] for the packages of the libraries `only` accepts (by folded name), and for the
/// instances (in creation order) up to the first one whose generic package body is not
/// analyzed yet: what can be done while the files of the other libraries are still to be
/// analyzed, in compile order, without loading a unit out of that order.
pub fn load_bodies_in(
    db: &mut Db,
    only: Option<&dyn Fn(&str) -> bool>,
) -> Result<(), Vec<Diagnostic>> {
    use yavs_sem::decl::Body;
    use yavs_sem::loader::{UnitKey, UnitSlot};
    let n_units = db.units.len();
    let pkgs: Vec<UnitId> = db
        .units
        .iter()
        .filter(|(_, u)| matches!(u.hir, UnitHir::Package { .. }))
        .filter(|(_, u)| db.libs[u.lib].name.key.as_str() != "std")
        .filter(|(_, u)| only.is_none_or(|f| f(db.libs[u.lib].name.key.as_str())))
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
    if only.is_some() && db.units.len() > n_units {
        return load_bodies_in(db, only);
    }
    if only.is_some() {
        use yavs_sem::ids::Id;
        for i in 0..db.insts.len() {
            let i = yavs_sem::InstId::from_index(i);
            if let DeclKind::Unit(u) = db.decls[db.insts[i].generic].kind {
                let (lib, key) = (db.units[u].lib, UnitKey::body(db.units[u].name.key));
                if !matches!(db.libs[lib].units.get(&key), Some(UnitSlot::Done(_))) {
                    break;
                }
            }
            if db.instance_body(i).is_err() {
                return Err(db.take_diagnostics());
            }
        }
        return Ok(());
    }
    // Bodies of generic package and subprogram instances (cloned from the generic body),
    // also of the instances a cloned body declares.
    let mut done = 0;
    while done < db.insts.len() {
        let insts: Vec<yavs_sem::InstId> = db.insts.iter().skip(done).map(|(i, _)| i).collect();
        done = db.insts.len();
        for i in insts {
            if db.instance_body(i).is_err() {
                return Err(db.take_diagnostics());
            }
        }
    }
    // A body can load more packages (fixed_generic_pkg's body uses math_real): visit them too.
    // Loads are memoized, so a second pass only does the new ones.
    if db.units.len() > n_units {
        return load_bodies_in(db, None);
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
    db.bind_package_paths();
    let pkg_uses = package_externals(db)?;
    let mut e = Elab {
        db,
        m: Model::default(),
        procs: Vec::new(),
        foreign_archs: Vec::new(),
        errors: Vec::new(),
        warnings: Vec::new(),
        cfg: cfg.clone(),
        stmt: None,
        pending: Vec::new(),
        idle_cache: HashMap::new(),
        idle_dead: false,
        deferred: Vec::new(),
        pkg_uses,
        units_seen: 0,
        parts_memo: None,
    };
    e.units_seen = e.db.units.len();
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
            .filter(|(_, u)| !builtin_lib(e.db, u.lib))
            .flat_map(|(u, _)| e.db.package_regions(u).unwrap_or_default())
            .flat_map(|s| e.db.decls_with_instances(s))
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
        // The shapes of the shared variables of loaded packages (waves, #321).
        let vars: Vec<DeclId> =
            e.db.units
                .iter()
                .filter(|(_, u)| !builtin_lib(e.db, u.lib))
                .flat_map(|(u, _)| e.db.package_regions(u).unwrap_or_default())
                .flat_map(|s| e.db.decls_with_instances(s))
                .filter(|&d| {
                    matches!(&e.db.decls[d].kind,
                    DeclKind::Object(o) if o.class == ObjClass::SharedVariable)
                })
                .collect();
        for d in vars {
            if let Err(x) = e.var_shape(None, d) {
                e.errors.push(*x);
            }
        }
        // Implicit signals named in the subprograms of a package body belong to the package
        // (LRM 16.2.4), after the signals they are of.
        let implicit: Vec<(UnitId, DeclId)> =
            e.db.units
                .iter()
                .filter_map(|(_, u)| match u.hir {
                    UnitHir::PackageBody { package } if !builtin_lib(e.db, u.lib) => {
                        Some((package, u.scope))
                    }
                    _ => None,
                })
                .flat_map(|(p, s)| e.db.scopes[s].order.iter().map(move |&d| (p, d)))
                .filter(|&(_, d)| {
                    matches!(&e.db.decls[d].kind,
                    DeclKind::Object(o) if o.role == ObjRole::ImplicitSignal)
                })
                .collect();
        for (unit, d) in implicit {
            let ps = e.m.pkg_scope(e.db, unit);
            if let Err(x) = e.implicit_signal(ps, d) {
                e.errors.push(*x);
            }
        }
    }
    // External names in processes refer to anything in the design (the reference
    // simulators bind them once the hierarchy exists, #93).
    if e.errors.is_empty()
        && let Err(x) = e.bind_late()
    {
        e.errors.push(*x);
    }
    if e.errors.is_empty() {
        e.check_sources();
    }
    if e.errors.is_empty() && e.cfg.collapse_ports {
        crate::collapse::ports(e.db, &mut e.m.nets, &e.m.sigs, &mut e.procs);
    }
    if !e.errors.is_empty() {
        let mut v = e.db.take_diagnostics();
        v.append(&mut e.warnings);
        v.append(&mut e.errors);
        return Err(v);
    }
    let forced = crate::collapse::forced(e.db, &e.m.sigs);
    e.m.nets.place(e.cfg.packs, forced.as_ref());
    let lean = crate::lean::leaves(e.db, &e.m.nets);
    let no_codes = crate::keep_codes::leaves(e.db, &e.m.nets);
    e.m.calls.flush();
    Ok(Elaborated {
        lean,
        no_codes,
        top,
        hier: e.m.hier,
        nets: e.m.nets,
        procs: e.procs,
        foreign_archs: e.foreign_archs,
        ext_procs: Vec::new(),
        convs: std::mem::take(&mut e.m.convs),
        model_vals: e.m.vals,
        sigs: e.m.sigs,
        regions: e.m.regions,
        var_ports: e.m.var_ports,
        ext_vars: e.m.ext_vars,
        ext_consts: e.m.ext_consts,
        pkg_ext: e.m.pkg_ext,
        ext_unbound: e.m.ext_unbound,
        shared_vars: e.m.shared_vars,
        var_shapes: e.m.var_shapes,
        ranges: e.m.ranges,
        warnings: e.warnings,
        ir: None,
        start: e.m.session.finish(),
        elab_calls: std::mem::take(&mut e.m.session.log),
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

    /// The architectures and entities loaded on demand bring packages with them, and an
    /// elaboration-time call of one of their subprograms needs the package body (#515, #516).
    fn load_new_bodies(&mut self, at: Span) -> R<()> {
        if self.db.units.len() == self.units_seen {
            return Ok(());
        }
        let r = load_bodies(self.db);
        self.units_seen = self.db.units.len();
        r.map_err(|mut ds| {
            let last = ds.pop();
            self.errors.extend(ds);
            Box::new(last.unwrap_or_else(|| ice(at, "body load failed without a diagnostic")))
        })
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
        self.load_new_bodies(at)?;
        // Generic types, subprograms and packages: elaborate the entity's clone for these
        // actuals, whose generics are the generic constants only.
        let (entity, arch, gens) = if self.db.has_non_constant_generics(entity) {
            self.instantiate(entity, arch, gens, at)?
        } else {
            (entity, arch, gens)
        };
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
        // A VHPI foreign architecture (#330): the plugin's model stands in for its
        // declarations and statements, which are not elaborated.
        let foreign = self.vhpi_arch(arch);
        // 4. Declarations of the entity, then of the architecture.
        let escope = self.db.units[entity].scope;
        let ascope = self.db.units[arch].scope;
        let regions: &[_] = if foreign.is_some() {
            &[escope]
        } else {
            &[escope, ascope]
        };
        for &region in regions {
            if let Err(d) = self.decls(s, region) {
                return Err(self.ctx(s, d));
            }
        }
        if let Some((library, model)) = foreign {
            self.foreign_archs.push(crate::ForeignArch {
                scope: s,
                library,
                model,
            });
            let mut counter = 0;
            self.stmts(s, &estmts, None, &mut counter, depth)?;
            return Ok(s);
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

    /// The library and model of architecture `arch` when its `'FOREIGN` value is a VHPI
    /// model (`"VHPI library model"` or `"VHPI library;model"`).
    fn vhpi_arch(&self, arch: UnitId) -> Option<(String, String)> {
        let key = Symbol::intern("foreign");
        let v = self.db.attr_index.get(&(self.db.units[arch].decl, key))?;
        match self.db.value(self.db.attr_values[*v].value)? {
            yavs_sem::stat::ConstValue::Array(a) => match &a.elems {
                yavs_sem::stat::Elems::Bytes(b) => {
                    let text: String = b.iter().map(|&c| char::from(c)).collect();
                    yavs_lower::vhpi_model(&text)
                }
                yavs_sem::stat::Elems::Runs(_) => None,
            },
            _ => None,
        }
    }

    /// The clone of `entity`/`arch` for the generic type, subprogram and package actuals in
    /// `gens`, and the associations of its generic constants.
    fn instantiate(
        &mut self,
        entity: UnitId,
        arch: UnitId,
        gens: Vec<A>,
        at: Span,
    ) -> R<(UnitId, UnitId, Vec<A>)> {
        let UnitHir::Entity { generics, .. } = &self.db.units[entity].hir else {
            return Err(Box::new(ice(at, "instance of a non-entity")));
        };
        let generics = generics.to_vec();
        let mut gens = gens.into_iter();
        let mut actuals = Vec::with_capacity(generics.len());
        let mut consts = Vec::new();
        for &g in &generics {
            let a = gens.next().unwrap_or(A::Default);
            match (&self.db.decls[g].kind, a) {
                (DeclKind::Object(_), a) => {
                    actuals.push(None);
                    consts.push(a);
                }
                (_, A::Generic(x)) => actuals.push(Some(x)),
                _ => return Err(no_generic_actual(self.db, g, at)),
            }
        }
        match self.db.instantiate_entity(entity, arch, &actuals, at) {
            Ok((e, a)) => Ok((e, a, consts)),
            Err(_) => Err(self.db_error(at)),
        }
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
            (_, A::Expr(e, _)) => {
                self.m.actual_of = Some(s);
                let v = self.m.eval(db, *e, gscope);
                self.m.actual_of = None;
                v?
            }
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
            (_, A::Generic(_)) => {
                return Err(Box::new(ice(
                    decl.span,
                    "generic type actual of a constant",
                )));
            }
            (_, A::Parts(_) | A::Conv { .. }) => {
                return Err(unsupported(
                    decl.span,
                    "individual or converted generic associations",
                    "M3",
                ));
            }
        };
        // An array generic of a constrained subtype takes that subtype's bounds, not the
        // actual's (implicit subtype conversion, LRM 6.5.6.2, 6.5.7.1).
        let v = self.m.as_subtype(db, o.subtype, v, Some(s), decl.span)?;
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
        let Some(&(leaf, range)) = rs.first() else {
            return Err(Box::new(ice(
                span,
                "implicit signal of a prefix without leaves",
            )));
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
        let src = (leaf, range);
        let db: &Db = self.db;
        let shape = match kind {
            ImplicitKind::Delayed(_) => pshape,
            _ => {
                let st = o.subtype;
                let Some(sh) = self.m.shape(db, st, Some(s), span)? else {
                    return Err(Box::new(ice(span, "implicit signal subtype")));
                };
                sh
            }
        };
        let obj = self
            .m
            .add_obj(self.db, s, d, ObjKind::Implicit, shape, None, span)?;
        let (a, b) = self.m.nets.obj(obj).leaves;
        if let ImplicitKind::Delayed(_) = kind {
            // A record prefix: one delayed leaf per leaf of the prefix, in leaf order.
            if (b.0 - a.0) as usize != rs.len() {
                return Err(Box::new(ice(
                    span,
                    "'delayed of another shape than its prefix",
                )));
            }
            for (k, &(sl, sr)) in rs.iter().enumerate() {
                let elem = self.m.nets.leaves[sl.index()].ty.elem;
                let init = self.m.nets.leaf(sl).init.clone();
                let l = &mut self.m.nets.leaves[a.index() + k];
                // The kernel decides the representation (K2): the prefix's for 'delayed.
                l.ty = yavs_kernel::SignalType { elem, len: sr.len };
                l.init = init;
                l.implicit = Some((kind, sl, sr));
            }
            return Ok(());
        }
        if b.0 != a.0 + 1 {
            return Err(Box::new(ice(span, "implicit signal of several leaves")));
        }
        let l = &mut self.m.nets.leaves[a.index()];
        // One logic element ('0'/'1' codes) for 'stable, 'quiet and 'transaction (K2), over
        // every leaf of the prefix.
        l.ty = yavs_kernel::SignalType::logic(1);
        l.implicit = Some((kind, src.0, src.1));
        l.implicit_more = rs[1..].to_vec();
        Ok(())
    }

    /// The implicit GUARD signal of a guarded block (LRM 11.2), driven by the block's guard
    /// process. It starts as the value of the guard condition (14.7.3.3, #464).
    fn guard_signal(&mut self, s: ScopeId, g: yavs_sem::DeclId) -> R<()> {
        let db: &Db = self.db;
        let decl = &db.decls[g];
        let DeclKind::Object(o) = &decl.kind else {
            return Err(Box::new(ice(decl.span, "GUARD is not an object")));
        };
        self.m.sig_init = true;
        let init = o.default.map(|e| self.m.eval(db, e, Some(s)));
        self.m.sig_init = false;
        let init = init.transpose()?;
        let Some(shape) = self
            .m
            .shape_init(db, o.subtype, init.as_ref(), Some(s), decl.span)?
        else {
            return Err(Box::new(ice(decl.span, "GUARD subtype")));
        };
        self.m
            .add_obj(db, s, g, ObjKind::Signal, shape, init.as_ref(), decl.span)?;
        Ok(())
    }

    /// Give the leaves of object `obj` the guard kind `g` and the disconnection time of
    /// declaration `d` (LRM 7.4): a guarded signal or a guarded (`bus`) port.
    fn set_guard(&mut self, s: ScopeId, d: yavs_sem::DeclId, obj: ObjId, g: GuardKind) -> R<()> {
        let db: &Db = self.db;
        let mut disconnect = yavs_kernel::Time::ZERO;
        if let Some(&e) = db.disconnect_times.get(&d) {
            let fs = self.m.eval(db, e, Some(s))?.as_int().ok_or_else(|| {
                Box::new(ice(
                    db.exprs[e].span,
                    "disconnection time is not an integer",
                ))
            })?;
            disconnect = yavs_kernel::Time::from_vhdl(fs).map_err(|_| {
                Box::new(
                    Diagnostic::error(
                        codes::EVAL_FAILED,
                        "disconnection time is negative or out of range",
                    )
                    .with_primary(db.exprs[e].span, "here"),
                )
            })?;
        }
        let (a, b) = self.m.nets.obj(obj).leaves;
        for l in &mut self.m.nets.leaves[a.index()..b.index()] {
            l.guard = Some(g);
            l.disconnect = disconnect;
        }
        Ok(())
    }

    /// Signal declarations of a declarative region (constants are evaluated on demand).
    fn decls(&mut self, s: ScopeId, region: yavs_sem::ScopeId) -> R<()> {
        if self.db.scopes[region].kind != yavs_sem::scope::ScopeKind::Process {
            self.m.regions.entry(s).or_default().push(region);
        }
        for d in self.db.decls_with_instances(region) {
            // External names in its subprograms bind once the hierarchy is elaborated, as
            // those of its statements (#255).
            for m in self.db.decl_subprograms(d) {
                let Some(sc) = self.db.sub_scope(m) else {
                    continue;
                };
                let hs = self.db.alias_external_uses(ExtOwner::Sub(m), sc);
                self.pending.extend(hs.iter().map(|&h| (s, h)));
            }
            if let DeclKind::Alias(Alias::Object { target, .. }) = self.db.decls[d].kind {
                // An alias of an external name, or of a part of one, binds here, in
                // elaboration order (LRM 8.7, 14.4.1).
                if let Some(x) = self.db.external_alias(d) {
                    self.m.bind(self.db, s, d, x)?;
                } else if !matches!(self.db.exprs[target].kind, ExprKind::Obj(_)) {
                    let hs = self
                        .db
                        .alias_external_uses(ExtOwner::Alias(d), region)
                        .to_vec();
                    for h in hs {
                        let x = self.db.implicit_external(h).expect("an external alias");
                        self.m.bind(self.db, s, h, x)?;
                    }
                }
                continue;
            }
            let db: &Db = self.db;
            let decl = &db.decls[d];
            // The ranges of a type or subtype that may differ per evaluation are evaluated
            // here, once and in declaration order; every use reads them (LRM 14.4.2.1, #612).
            // A process's are its lowered code's.
            if let DeclKind::Type(_) | DeclKind::Subtype(_) = decl.kind
                && db.scopes[region].kind != yavs_sem::scope::ScopeKind::Process
            {
                for rc in db.declared_ranges(d) {
                    if let Some(left) = db.dynamic_range(rc) {
                        // A signal reads as its initial value, as in a constant (#453).
                        self.m.sig_init = true;
                        let b = self.m.range(db, rc, Some(s), decl.span);
                        self.m.sig_init = false;
                        let b = b.map_err(|x| {
                            Box::new(x.with_note(format!(
                                "while elaborating the declaration of `{}` (LRM 14.4.2.1)",
                                decl.name
                            )))
                        })?;
                        self.m.ranges.insert((s, left), b);
                    }
                }
                continue;
            }
            let DeclKind::Object(o) = &decl.kind else {
                continue;
            };
            // A constant nothing reads is never evaluated, but the constraint of its subtype
            // indication is elaborated with the declaration (LRM 5.3.2.2, 14.4.2.1): its shape
            // checks the index ranges (#825). An impure bound is left to the evaluation.
            if o.class == ObjClass::Constant
                && o.role == ObjRole::Plain
                && db.scopes[region].kind != yavs_sem::scope::ScopeKind::Process
                && db.subtypes[o.subtype].name.is_none()
            {
                let rs = db.subtype_ranges(o.subtype);
                let unfolded = rs.iter().any(|r| {
                    matches!(r, yavs_sem::hir::RangeC::Explicit { left, right, .. }
                        if db.value(*left).is_none() || db.value(*right).is_none())
                });
                if unfolded && rs.iter().all(|r| db.dynamic_range(r).is_none()) {
                    match self.m.shape(db, o.subtype, Some(s), decl.span) {
                        Err(x) if x.code != codes::NOT_EVALUATED => return Err(x),
                        _ => {}
                    }
                }
            }
            match (o.class, &o.role) {
                // The actual of a variable port finds its object here (#188).
                (ObjClass::SharedVariable, _) => {
                    self.m.shared_vars.insert((s, d));
                    self.var_shape(Some(s), d)?;
                }
                (ObjClass::Variable, ObjRole::Plain)
                    if db.scopes[region].kind == yavs_sem::scope::ScopeKind::Process =>
                {
                    self.var_shape(Some(s), d)?;
                }
                (ObjClass::Signal, ObjRole::Plain) => {
                    let span = decl.span;
                    self.m.sig_init = true;
                    let init = o.default.map(|e| self.m.eval(db, e, Some(s)));
                    self.m.sig_init = false;
                    let init = init.transpose()?;
                    let Some(shape) =
                        self.m
                            .shape_init(db, o.subtype, init.as_ref(), Some(s), span)?
                    else {
                        return Err(unconstrained_signal(span));
                    };
                    let guard = o.signal_kind.map(|k| match k {
                        yavs_syntax::ast::SignalKind::Bus => GuardKind::Bus,
                        yavs_syntax::ast::SignalKind::Register => GuardKind::Register,
                    });
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
                        self.set_guard(s, d, obj, g)?;
                    }
                }
                // A guarded (`bus`) port is a guarded signal too: its leaves get the
                // guard kind and the disconnection time, as a plain signal's (#656).
                (ObjClass::Signal, ObjRole::Port) => {
                    let Some(g) = o.signal_kind.map(|k| match k {
                        yavs_syntax::ast::SignalKind::Bus => GuardKind::Bus,
                        yavs_syntax::ast::SignalKind::Register => GuardKind::Register,
                    }) else {
                        continue;
                    };
                    if let Some(obj) = self.m.signal(s, d) {
                        self.set_guard(s, d, obj, g)?;
                    }
                }
                (ObjClass::Signal, ObjRole::Guard) => {
                    // The implicit GUARD signal of a guarded block (LRM 11.2), driven by
                    // the block's guard process.
                    self.guard_signal(s, d)?;
                }
                (ObjClass::Signal, ObjRole::ImplicitSignal) => {
                    self.implicit_signal(s, d)?;
                }
                // A constant whose initial value calls an impure user function is evaluated
                // here, in declaration order, so its side effects on package state happen once
                // and in order (LRM 14.4.2); other constants are evaluated when first needed.
                // A process's declarative part is elaborated by the process itself when it
                // starts (its lowered code): evaluating it here too ran the calls twice (#251).
                // One initialised from an external name binds it now (LRM 8.7, 14.4.1).
                (ObjClass::Constant, ObjRole::Plain)
                    if db.scopes[region].kind != yavs_sem::scope::ScopeKind::Process
                        && o.default.is_some_and(|e| db.uses_external(e)) =>
                {
                    self.m.value_of(db, d, Some(s), decl.span)?;
                }
                // A constant that reads a signal gets the signal's initial value, as the initial
                // value of a signal does (#453). One that cannot be evaluated yet keeps its
                // run-time thunk.
                (ObjClass::Constant, ObjRole::Plain)
                    if db.scopes[region].kind != yavs_sem::scope::ScopeKind::Process
                        && o.default.is_some_and(|e| crate::eval::reads_signal(db, e)) =>
                {
                    self.m.sig_init = true;
                    let r = self.m.value_of(db, d, Some(s), decl.span);
                    self.m.sig_init = false;
                    match r {
                        Ok(_) => {}
                        Err(x) if x.code == crate::codes::NOT_EVALUATED => {}
                        Err(x) => return Err(x),
                    }
                }
                (ObjClass::Constant, ObjRole::Plain)
                    if db.scopes[region].kind != yavs_sem::scope::ScopeKind::Process
                        && o.default.is_some_and(|e| crate::eval::calls_impure(db, e)) =>
                {
                    let mark = self.m.session.mark();
                    match self.m.value_of(db, d, Some(s), decl.span) {
                        Ok(_) => {}
                        // ponytail: a value elaboration cannot evaluate yet (a null
                        // multi-dimensional aggregate, a call with a signal parameter, say) is
                        // left to the constant's run-time thunk, which runs after the
                        // elaboration-time calls of every region: W2118 says so. The calls
                        // made before the failure are undone so that none runs twice.
                        Err(x) if x.code == crate::codes::NOT_EVALUATED => {
                            self.m.session.undo(db, mark).map_err(|m| {
                                Box::new(ice(decl.span, &format!("undoing calls: {m}")))
                            })?;
                            self.warnings.push(
                                Diagnostic::warning(
                                    codes::IMPURE_AT_RUN_TIME,
                                    format!(
                                        "constant `{}` calls an impure function but is \
                                         evaluated when the simulation starts",
                                        decl.name
                                    ),
                                )
                                .with_primary(decl.span, "not evaluated during elaboration")
                                .with_note(format!("during elaboration: {}", x.message))
                                .with_note(
                                    "its calls run after the elaboration-time calls of every \
                                     region, not in declaration order (LRM 14.4.2)",
                                ),
                            );
                        }
                        Err(x) => return Err(x),
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// A 2019 variable port (LRM 6.5.2) denotes its actual's protected object: the shared
    /// variable at the end of the chain of variable ports (#188).
    fn var_port(
        &mut self,
        s: ScopeId,
        p: DeclId,
        a: &A,
        ascope: Option<ScopeId>,
        at: Span,
    ) -> R<()> {
        let db: &Db = self.db;
        let decl = &db.decls[p];
        let actual = match (a, ascope) {
            (A::Expr(e, false), Some(sc)) => match db.exprs[*e].kind {
                ExprKind::Obj(d) => Some((sc, d, db.exprs[*e].span)),
                _ => None,
            },
            (A::Local(d), Some(sc)) => Some((sc, *d, at)),
            (A::Open, None) => {
                // Sem rejects an unassociated variable port of an instance; the top level
                // has nothing to associate (6.5.6.3).
                return Err(Box::new(
                    Diagnostic::error(
                        codes::VARIABLE_PORT_OPEN,
                        format!("top-level variable port `{}` has no actual", decl.name),
                    )
                    .with_primary(decl.span, "a variable port denotes its actual's object")
                    .with_help("instantiate this entity with a shared variable as the actual"),
                ));
            }
            _ => None,
        };
        let Some((sc, d, aspan)) = actual else {
            return Err(Box::new(ice(
                at,
                "variable port actual is not an object name",
            )));
        };
        let mut cur = Some(sc);
        let mut obj = None;
        while let (Some(r), None) = (cur, obj) {
            obj = match self.m.var_ports.get(&(r, d)) {
                Some(&v) => Some(v),
                None if self.m.shared_vars.contains(&(r, d)) => {
                    Some(VarObj::Region { region: r, decl: d })
                }
                None => None,
            };
            cur = self.m.hier.scope(r).parent;
        }
        use yavs_sem::scope::ScopeKind as S;
        let obj = match obj {
            Some(v) => v,
            None if matches!(
                db.scopes[db.decls[d].scope].kind,
                S::Package | S::PackageBody
            ) =>
            {
                VarObj::Package(d)
            }
            None => {
                return Err(Box::new(ice(
                    aspan,
                    "the actual's shared variable is not elaborated",
                )));
            }
        };
        self.m.var_ports.insert((s, p), obj);
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
            return self.var_port(s, p, a, ascope, at);
        }
        let component = matches!(self.m.hier.scope(s).kind, ScopeKind::Component { .. });
        let Some(hmode) = o.mode else {
            return Err(Box::new(ice(span, "port without a mode")));
        };
        // A mode view (2019) gives each record element its own mode; the object's mode is
        // only a summary, the port edges take each leaf's (ir.md 4, H10).
        let mode = match hmode {
            Mode::View { .. } => PortMode::Inout,
            m => port_mode(m),
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
            (A::Expr(e, inertial), Some(sc)) => match self.m.name_ref(self.db, *e, sc)? {
                // `inertial` makes even a signal name an expression actual (LRM 6.5.6.3).
                Some(n) if !*inertial => Some((n, db.exprs[*e].span)),
                _ => {
                    if mode != PortMode::In {
                        return Err(Box::new(ice(
                            db.exprs[*e].span,
                            "expression actual of a non-in port",
                        )));
                    }
                    // 2008 expression actual: a static value is the port's value; any other
                    // drives the port through an equivalent process (LRM 6.5.6.3).
                    // The actual's context subtype may name the instance's generics (#475).
                    self.m.actual_of = Some(s);
                    let v = match self.m.eval(db, *e, Some(sc)) {
                        Ok(v) => v,
                        Err(d) if d.code == codes::NOT_EVALUATED => {
                            let espan = db.exprs[*e].span;
                            let shape = match own {
                                Some(sh) => sh,
                                None => self
                                    .m
                                    .shape(db, db.exprs[*e].subtype, Some(sc), espan)?
                                    .ok_or_else(|| not_static_subtype(espan, span, decl))?,
                            };
                            let fo =
                                self.m
                                    .add_obj(self.db, s, p, kind, shape, init.as_ref(), span)?;
                            self.m.actual_of = None;
                            return self.actual_process(s, sc, fo, Vec::new(), *e, Some(subtype));
                        }
                        Err(d) => return Err(d),
                    };
                    self.m.actual_of = None;
                    // An unconstrained formal takes the bounds of the static value.
                    let espan = db.exprs[*e].span;
                    let shape = match own {
                        Some(sh) => sh,
                        None => self
                            .m
                            .shape_init(db, subtype, Some(&v), Some(s), espan)?
                            .ok_or_else(|| not_static_subtype(espan, span, decl))?,
                    };
                    self.m
                        .add_obj(self.db, s, p, kind, shape, Some(&v), espan)?;
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
                // An unconstrained formal takes the bounds of a conversion (LRM 5.3.2.2): the
                // result subtype or type mark of the actual part's (modes in, inout), else the
                // parameter subtype or type mark of the formal part's; sem made sure it has them.
                let conv_st = |c: Option<DeclId>, result: bool| {
                    c.and_then(|c| match &db.decls[c].kind {
                        DeclKind::Subtype(st) => Some(*st),
                        DeclKind::Type(t) => Some(db.types[*t].first_subtype),
                        k => k.subprogram().and_then(|sp| match result {
                            true => sp.ret,
                            false => sp.params.first().map(|p| p.subtype),
                        }),
                    })
                };
                let by = match mode {
                    PortMode::In => conv_st(*actual_conv, true),
                    PortMode::Inout => {
                        conv_st(*actual_conv, true).or_else(|| conv_st(*formal_conv, false))
                    }
                    PortMode::Out | PortMode::Buffer => conv_st(*formal_conv, false),
                };
                let own = match (own, by) {
                    (None, Some(st)) => self.m.shape(db, st, Some(sc), aspan)?,
                    (o, _) => o,
                };
                let Some(shape) = own else {
                    return Err(Box::new(ice(
                        span,
                        "a converted unconstrained port without bounds from its conversion",
                    )));
                };
                let fshape = shape.clone();
                let fo = self
                    .m
                    .add_obj(self.db, s, p, kind, shape, init.as_ref(), span)?;
                let fl = self.m.select(fo, &[], span)?.0;
                let db: &Db = self.db;
                // `actual_conv(actual)` feeds an `in`/`inout` formal, `formal_conv(formal)`
                // its actual.
                let idx = |m: &mut crate::Model,
                           f: Option<DeclId>,
                           from: &Shape,
                           to: &Shape|
                 -> R<Option<u32>> {
                    match f {
                        None => Ok(None),
                        Some(f) => {
                            let c = crate::eval::conv_fn(db, f, from, to, aspan)?;
                            m.convs.push(c);
                            Ok(Some(m.convs.len() as u32 - 1))
                        }
                    }
                };
                let to_formal = idx(&mut self.m, *actual_conv, &ashape, &fshape)?;
                let to_actual = idx(&mut self.m, *formal_conv, &fshape, &ashape)?;
                let ([f], [a]) = (fl.as_slice(), al.as_slice()) else {
                    // Several leaves on a side: one kernel converter over all of them, so
                    // the conversion runs once per cycle with every part current.
                    let one_way = match mode {
                        PortMode::In => to_formal.is_none(),
                        PortMode::Out | PortMode::Buffer => to_actual.is_none(),
                        // The formal side alone (L72): copy-in passes the actual unchanged.
                        PortMode::Inout => to_actual.is_none(),
                    };
                    if one_way {
                        // Sem rejects the other modes and the actual side alone.
                        return Err(Box::new(ice(
                            aspan,
                            "a composite port converted in one direction only",
                        )));
                    }
                    self.m.nets.conv_ports.push(crate::hier::ConvPort {
                        formal: fl,
                        actual: al,
                        mode,
                        to_formal,
                        to_actual,
                    });
                    return Ok(());
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
                let shape = match own {
                    Some(sh) => sh,
                    None => {
                        let ps: Vec<_> = parts.iter().map(|(p, e, ..)| (&p[..], *e)).collect();
                        self.m.parts_shape(self.db, s, subtype, &ps, sc, decl)?
                    }
                };
                let fo = self
                    .m
                    .add_obj(self.db, s, p, kind, shape, init.as_ref(), span)?;
                for (steps, e, _, inertial, conv) in parts {
                    let espan = self.db.exprs[*e].span;
                    let fsels = self.m.steps(self.db, None, steps, sc, espan)?;
                    let (fl, fshape) = self.m.select(fo, &fsels, espan)?;
                    if *conv {
                        // `f.x => conv(s)`: a port edge that converts, as for a whole formal
                        // (LRM 6.5.7.1, #611).
                        let db: &Db = self.db;
                        let (c, x) = match &db.exprs[*e].kind {
                            ExprKind::Call { callee, args, .. } => match args.first() {
                                Some(Arg {
                                    actual: Actual::Expr(x),
                                    ..
                                }) => (*callee, *x),
                                _ => {
                                    return Err(Box::new(ice(espan, "conversion without operand")));
                                }
                            },
                            ExprKind::Conv { to, operand } => {
                                (db.types[db.base(*to)].decl, *operand)
                            }
                            _ => {
                                return Err(Box::new(ice(
                                    espan,
                                    "part conversion of another kind",
                                )));
                            }
                        };
                        let Some((ao, asels)) = self.m.name_ref(db, x, sc)? else {
                            return Err(Box::new(ice(
                                espan,
                                "converted part actual without a signal",
                            )));
                        };
                        let (al, ashape) = self.m.select(ao, &asels, espan)?;
                        let k = crate::eval::conv_fn(db, c, &ashape, &fshape, espan)?;
                        self.m.convs.push(k);
                        let to_formal = Some(self.m.convs.len() as u32 - 1);
                        match (fl.as_slice(), al.as_slice()) {
                            ([f], [a]) => self.m.nets.ports.push(PortEdge {
                                formal: *f,
                                actual: *a,
                                mode,
                                to_formal,
                                to_actual: None,
                            }),
                            _ => self.m.nets.conv_ports.push(crate::hier::ConvPort {
                                formal: fl,
                                actual: al,
                                mode,
                                to_formal,
                                to_actual: None,
                            }),
                        }
                        continue;
                    }
                    // `inertial` makes even a signal name an expression actual (LRM 6.5.6.3).
                    let name = match self.m.name_ref(self.db, *e, sc)? {
                        Some(_) if *inertial => None,
                        n => n,
                    };
                    let Some((ao, asels)) = name else {
                        if mode != PortMode::In {
                            return Err(Box::new(ice(espan, "expression actual of a non-in port")));
                        }
                        // A static value is the part's initial value; any other expression
                        // drives the part through an equivalent process (LRM 6.5.6.3).
                        self.m.actual_of = Some(s);
                        let r = self.m.eval(self.db, *e, Some(sc));
                        self.m.actual_of = None;
                        match r {
                            Ok(v) => self.m.part_init(self.db, &fl, &fshape, &v, espan)?,
                            Err(d) if d.code == codes::NOT_EVALUATED => {
                                // A scalar part's subtype, for its range check: from the
                                // elaborated port, whose subtype a cloned instance has bound.
                                let part = scalar_part(self.db, subtype, steps);
                                self.actual_process(s, sc, fo, fsels, *e, part)?
                            }
                            Err(d) => return Err(d),
                        }
                        continue;
                    };
                    let (al, _) = self.m.select(ao, &asels, espan)?;
                    let modes = self.m.leaf_modes(self.db, fo, hmode, span)?;
                    self.m.edges(&fl, &al, &modes, espan)?;
                }
                return Ok(());
            }
            _ => None,
        };
        let Some(((ao, asels), aspan)) = actual else {
            // open or default; an unconstrained `in` port takes the bounds of its default
            // (#218, docs/design/lrm-checks.md L59)
            let own = match (own, &init) {
                (None, Some(v)) if mode == PortMode::In => {
                    self.m.shape_init(db, subtype, Some(v), Some(s), span)?
                }
                (o, _) => o,
            };
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
        let modes = self.m.leaf_modes(self.db, fo, hmode, span)?;
        self.m.edges(&fl, &al, &modes, aspan)
    }

    /// The equivalent process `formal <= e` of an expression actual (LRM 6.5.6.3) in the
    /// instantiating region `region`: it drives part `fsels` of formal `fo` of instance (or
    /// block) `s`, one delta after the expression changes. Lowered by yavs-lower
    /// (`Lowerer::port_actual`; `formal`, the subtype of the formal or of the part, for the
    /// range check of a scalar).
    fn actual_process(
        &mut self,
        s: ScopeId,
        region: ScopeId,
        fo: ObjId,
        fsels: Vec<Sel>,
        e: ExprId,
        formal: Option<yavs_sem::SubtypeId>,
    ) -> R<()> {
        let span = self.db.exprs[e].span;
        let shape = self.m.select(fo, &fsels, span)?.1;
        let Some(st) = self.stmt else {
            return Err(Box::new(ice(span, "port map outside a statement")));
        };
        let n = Symbol::intern(&format!(
            "{}.{}'actual",
            self.m.hier.scope(s).name.text,
            self.m.nets.obj(fo).name.text
        ));
        let name = Ident {
            text: n,
            key: n,
            span,
        };
        // A scalar formal whose range comes from the instance's generics: its bounds,
        // evaluated in the instance, for the process's range check.
        let db: &Db = self.db;
        let bounds = match formal {
            Some(f) if db.static_bounds(f).is_none() && db.types[db.base(f)].kind.is_scalar() => {
                Some(self.formal_bounds(f, s, span)?)
            }
            _ => None,
        };
        self.process_refs(region, st, name, false, vec![(fo, fsels, span)])?;
        // External names in the expression bind once the hierarchy is complete (#93).
        if let Some(r) = self.m.regions.get(&region).and_then(|v| v.last().copied()) {
            let hs = self.db.alias_external_uses(ExtOwner::Expr(e), r);
            self.pending.extend(hs.iter().map(|&h| (region, h)));
        }
        self.procs.last_mut().expect("just added").actual = Some(Box::new(crate::PortActual {
            expr: e,
            formal,
            port: self.m.nets.obj(fo).decl,
            shape,
            bounds,
        }));
        Ok(())
    }

    /// The range of scalar subtype `f` evaluated in scope `s`: left, right (f64 bits for a
    /// real) and the direction (0 for `to`), as `yavs_lower::PortActual::bounds` reads them.
    fn formal_bounds(&mut self, f: yavs_sem::SubtypeId, s: ScopeId, span: Span) -> R<[u64; 3]> {
        use yavs_sem::hir::{Direction, RangeC};
        let db: &Db = self.db;
        let dir = |d: Direction| u64::from(d == Direction::Downto);
        if !matches!(
            db.types[db.base(f)].kind,
            yavs_sem::types::TypeKind::Floating | yavs_sem::types::TypeKind::UniversalReal
        ) {
            let b = self.m.scalar_bounds(db, f, Some(s), span)?;
            return Ok([b.left as u64, b.right as u64, dir(b.dir)]);
        }
        let Some(RangeC::Explicit {
            left,
            dir: d,
            right,
        }) = db.scalar_range(f)
        else {
            return Err(unsupported(
                span,
                "a real port range given by an attribute",
                "M3",
            ));
        };
        let mut real = |e: ExprId| match self.m.eval(db, e, Some(s))? {
            Val::Real(x) => Ok(x.to_bits()),
            _ => Err(Box::new(ice(
                db.exprs[e].span,
                "real bound of another type",
            ))),
        };
        Ok([real(left)?, real(right)?, dir(d)])
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
        self.stmt = Some(st);
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
                // Implicit signals ('stable, ...) named in the process belong to its enclosing
                // region (LRM 16.2.4), where the region's instance data finds them.
                if let Some(r) = pscope {
                    self.decls(s, r)?;
                }
                // External names used in the statements bind once the hierarchy is complete.
                let region = pscope.or_else(|| self.m.regions.get(&s)?.last().copied());
                if let Some(region) = region {
                    let hs = self.db.alias_external_uses(ExtOwner::Stmt(st), region);
                    self.pending.extend(hs.iter().map(|&h| (s, h)));
                }
                Ok(())
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
                let bcfg = match cfg {
                    Some(c) => self.sub_cfg(c, st, None, b)?,
                    None => None,
                };
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
                        self.guard_signal(b, g)?;
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
                    let reads = self.guard_reads(b, g);
                    if let Some(p) = self.procs.last_mut() {
                        p.guard = reads;
                    }
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
                    let gcfg = match cfg {
                        Some(c) => self.sub_cfg(c, st, Some(&val), g)?,
                        None => None,
                    };
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
                        let alt = alabel;
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
                            Choice::Expr(e) => {
                                crate::value::same(&self.m.eval(db, *e, Some(s))?, &v)
                            }
                            Choice::Range(r) => {
                                let b = self.m.range(db, r, Some(s), span)?;
                                v.as_int().is_some_and(|x| b.pos(x).is_some())
                            }
                            Choice::Element(_) => false,
                        };
                        if hit {
                            pick = Some((alt.scope, alt.body.to_vec(), alt.label));
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
        alt: Option<Ident>,
        depth: u32,
    ) -> R<()> {
        let g = self.m.hier.add(
            Some(s),
            ScopeKind::Generate {
                alt: alt.map(|l| l.text),
            },
            label,
        );
        let gcfg = cfg.and_then(|c| self.sub_cfg_alt(c, st, alt.map(|l| l.key)));
        self.decls(g, gscope)?;
        let mut c = 0;
        self.stmts(g, body, gcfg.as_ref(), &mut c, depth)
    }

    /// The block configuration item for block or generate statement `st` (with the
    /// generate parameter value `val`); a generate index is evaluated in `scope`, the
    /// scope of the block or generate being entered (#41).
    fn sub_cfg(
        &mut self,
        c: &CfgPath,
        st: StmtId,
        val: Option<&Val>,
        scope: ScopeId,
    ) -> R<Option<CfgPath>> {
        let db: &Db = self.db;
        let Some(b) = block_cfg(db, c) else {
            return Ok(None);
        };
        for (i, it) in b.items.iter().enumerate() {
            let ConfigItem::Block(bc) = it else { continue };
            let hit = match &bc.target {
                BlockTarget::Block(s) => *s == st,
                BlockTarget::Generate { stmt, select } => {
                    *stmt == st
                        && match (select, val) {
                            (None, _) => true,
                            (Some(GenSelect::Index(e)), Some(v)) => {
                                self.m.eval(db, *e, Some(scope))? == *v
                            }
                            (Some(GenSelect::Range(r)), Some(v)) => {
                                let b = self.m.range(db, r, Some(scope), bc.span)?;
                                v.as_int().is_some_and(|x| b.pos(x).is_some())
                            }
                            _ => false,
                        }
                }
                BlockTarget::Architecture(_) => false,
            };
            if hit {
                let mut p = c.clone();
                p.steps.push(i);
                return Ok(Some(p));
            }
        }
        Ok(None)
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
        // Incremental binding: its associations replace the primary's (sim1's permissive
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
        // Generic types, subprograms and packages: elaborate the component's clone for
        // these actuals; the binding refers to the original's formals (#222).
        let mut formals: HashMap<DeclId, GenericActual> = HashMap::new();
        let (cgens, cports, g, orig) = if cgens.iter().any(|&d| self.db.is_non_constant_generic(d))
        {
            let mut actuals = Vec::with_capacity(cgens.len());
            let mut consts = Vec::new();
            for (&d, a) in cgens.iter().zip(g) {
                match a {
                    A::Generic(x) if self.db.is_non_constant_generic(d) => {
                        formals.insert(d, x);
                        actuals.push(Some(x));
                    }
                    _ if self.db.is_non_constant_generic(d) => {
                        return Err(no_generic_actual(self.db, d, span));
                    }
                    a => {
                        consts.push(a);
                        actuals.push(None);
                    }
                }
            }
            let n = match self.db.instantiate_component(comp, &actuals, span) {
                Ok(n) => n,
                Err(_) => return Err(self.db_error(span)),
            };
            let DeclKind::Component(nc) = &self.db.decls[n].kind else {
                return Err(Box::new(ice(span, "component clone")));
            };
            let orig: Vec<DeclId> = cgens
                .iter()
                .copied()
                .filter(|&d| !self.db.is_non_constant_generic(d))
                .chain(cports.iter().copied())
                .collect();
            (nc.generics.to_vec(), nc.ports.to_vec(), consts, orig)
        } else {
            (cgens.to_vec(), cports.to_vec(), g, Vec::new())
        };
        for (i, &cg) in cgens.iter().enumerate() {
            self.generic(c, cg, &g[i], Some(s), &[], span)?;
        }
        for (i, &cp) in cports.iter().enumerate() {
            self.port(c, cp, &p[i], Some(s), span)?;
        }
        // The original's generic constants and ports name the clone's objects.
        for (&o, &n) in orig.iter().zip(cgens.iter().chain(&cports)) {
            if let Some(v) = self.m.vals.get(c.index()).and_then(|m| m.get(&n)).cloned() {
                self.m.set(c, o, v);
            }
            if let Some(x) = self.m.signal(c, n) {
                self.m.sigs.insert((c, o), x);
            }
        }
        match self.binding(region, st, comp, cfg, span)? {
            Some(mut b) => {
                if !formals.is_empty() {
                    for a in &mut b.gmap {
                        self.through_component(a, &formals, span)?;
                    }
                }
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

    /// An entity generic's actual in a binding (default by name, or a generic map) that
    /// names a generic type, subprogram or package of the component: the component's actual.
    fn through_component(
        &self,
        a: &mut A,
        formals: &HashMap<DeclId, GenericActual>,
        span: Span,
    ) -> R<()> {
        let db: &Db = self.db;
        let formal = match &*a {
            A::Local(l) => Some(*l),
            A::Generic(GenericActual::Subprogram(d)) => Some(*d),
            A::Generic(GenericActual::Subtype(st)) => {
                let t = db.base(*st);
                match formals
                    .keys()
                    .find(|&&f| matches!(db.decls[f].kind, DeclKind::GenericType(x) if x == t))
                {
                    Some(&f) if db.types[t].first_subtype == *st => Some(f),
                    Some(_) => {
                        return Err(unsupported(
                            span,
                            "a subtype of a component's generic type as the actual of an entity's",
                            "#222",
                        ));
                    }
                    None => None,
                }
            }
            A::Generic(GenericActual::Package(i)) => formals
                .keys()
                .find(|&&f| matches!(db.decls[f].kind, DeclKind::GenericPackage(x) if x == *i))
                .copied(),
            _ => None,
        };
        if let Some(x) = formal.and_then(|f| formals.get(&f)) {
            *a = A::Generic(*x);
        }
        Ok(())
    }

    /// The last diagnostic Db reported (the others go to `errors`).
    fn db_error(&mut self, at: Span) -> Box<Diagnostic> {
        let mut ds = self.db.take_diagnostics();
        let last = ds
            .pop()
            .map(Box::new)
            .unwrap_or_else(|| Box::new(ice(at, "instantiation failed without a diagnostic")));
        self.errors.extend(ds);
        last
    }

    /// The shape of variable `d` of region `s` (`None`: a package's) for the waveform
    /// recorder (`Elaborated::var_shapes`, #321), from its subtype. Left out when elaboration cannot compute it without calling a function the
    /// process calls itself when it starts (#251): such a call is undone.
    fn var_shape(&mut self, s: Option<ScopeId>, d: DeclId) -> R<()> {
        let db: &Db = self.db;
        let decl = &db.decls[d];
        let DeclKind::Object(o) = &decl.kind else {
            return Ok(());
        };
        let k = &db.types[db.base(o.subtype)].kind;
        if !(k.is_scalar()
            || matches!(
                k,
                yavs_sem::types::TypeKind::Array { .. } | yavs_sem::types::TypeKind::Record { .. }
            ))
        {
            return Ok(());
        }
        let mark = self.m.session.mark();
        let sh = self.m.shape(db, o.subtype, s, decl.span).ok().flatten();
        let called = self.m.session.mark() != mark;
        self.m
            .session
            .undo(db, mark)
            .map_err(|m| Box::new(ice(decl.span, &format!("undoing calls: {m}"))))?;
        if let (Some(sh), false) = (sh, called) {
            self.m.var_shapes.insert((s, d), sh);
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
        if lsps.iter().any(|l| matches!(l.root, ObjRoot::External(_))) {
            // Drivers of external names bind once the hierarchy is complete (#93).
            let idx = self.add_process(s, st, name, postponed);
            self.deferred.push((idx, s, lsps.to_vec()));
            return Ok(());
        }
        let mut refs = Vec::with_capacity(lsps.len());
        for l in lsps {
            let (o, sels) = self.m.lsp_ref(self.db, l, s)?;
            refs.push((o, sels, l.span));
        }
        self.process_refs(s, st, name, postponed, refs)?;
        let trigger = self.edge_trigger(s, st);
        let idle = match trigger {
            Some(_) => self.idle_reads(s, st),
            None => self.idle_comb_reads(s, st),
        };
        let copy = self.copy_source(s, st);
        let toggle = self.toggle_source(s, st);
        let gate = self.gate_source(s, st);
        let mux = self.mux_source(s, st);
        let prog = if copy.is_none() && toggle.is_none() && gate.is_none() && mux.is_none() {
            self.prog_source(s, st)
        } else {
            None
        };
        if let Some(p) = self.procs.last_mut() {
            p.idle = idle;
            p.trigger = trigger;
            p.copy = copy;
            p.toggle = toggle;
            p.gate = gate;
            p.mux = mux;
            p.prog = prog;
        }
        Ok(())
    }

    /// The source of a concurrent `target <= source;` that copies one signal to another: no
    /// delay clause (or `transport`, or `inertial` without `reject`), not guarded or postponed,
    /// one waveform element without `after`, target and source static signal names whose
    /// elaborated shapes have the same types, lengths and scalar subtypes or bounds (so no check
    /// of the assignment can fail), and source parts that match the target's (just set by
    /// `process_refs`) one to one in length and element kind, each target part a whole driver
    /// of the process. Its parts in element order; `None` for anything else.
    fn copy_source(&mut self, s: ScopeId, st: StmtId) -> Option<Vec<(LeafId, Range)>> {
        let db: &Db = &*self.db;
        let StmtKind::ConcSignalAssign {
            postponed: false,
            guarded: false,
            target,
            delay: Delay::Default | Delay::Transport | Delay::Inertial { reject: None },
            rhs: Rhs::Simple(Waveform::Elements(elems)),
            ..
        } = &db.stmts[st].kind
        else {
            return None;
        };
        let [
            WaveElem {
                value: Some(e),
                after: None,
            },
        ] = &elems[..]
        else {
            return None;
        };
        let (e, t) = (*e, target.expr);
        self.copy_parts(s, e, t)
    }

    /// The parts of the static signal name `e` (or a conversion of one between arrays of one
    /// element type) that a copy to target `t` reads: `copy_source` without the statement.
    fn copy_parts(&mut self, s: ScopeId, e: ExprId, t: ExprId) -> Option<Vec<(LeafId, Range)>> {
        let db: &Db = &*self.db;
        // A conversion between arrays of one element type (`std_logic_vector(u(7 downto 0))`) moves
        // no value: the copy is positional.
        let conv = match &db.exprs[e].kind {
            ExprKind::Conv { to, operand }
                if matches!(db.types[db.base(*to)].kind, TypeKind::Array { .. })
                    && matches!(
                        db.types[db.base(db.exprs[*operand].subtype)].kind,
                        TypeKind::Array { .. }
                    )
                    && db.constrainedness(*to) == Constrainedness::Unconstrained =>
            {
                Some(*operand)
            }
            _ => None,
        };
        let (src, sshape) = self.static_signal_parts(s, conv.unwrap_or(e))?;
        let (_, tshape) = self.static_signal_parts(s, t)?;
        let same = if conv.is_some() {
            same_array_values(&sshape, &tshape)
        } else {
            same_values(&sshape, &tshape)
        };
        if !same {
            return None;
        }
        let p = self.procs.last()?;
        let [dst] = &p.lsps[..] else {
            return None;
        };
        let nets = &self.m.nets;
        let ok = src.len() == dst.len()
            && src.iter().zip(dst).all(|(&(sl, sr), &(k, dl, dr))| {
                sr.len == dr.len
                    && nets.leaf(sl).ty.elem == nets.leaf(dl).ty.elem
                    && nets.drivers[k].leaf == dl
                    && nets.drivers[k].range == dr
            });
        ok.then_some(src)
    }

    /// The leaves and shape of `e` when it is a static name of a signal: an object, indexed,
    /// sliced and selected by static expressions only.
    fn static_signal_parts(&mut self, s: ScopeId, e: ExprId) -> SigParts {
        if let Some(m) = &self.parts_memo
            && let Some(v) = m.get(&e)
        {
            return v.clone();
        }
        let v = self.static_signal_parts_now(s, e);
        if let Some(m) = &mut self.parts_memo {
            m.insert(e, v.clone());
        }
        v
    }

    fn static_signal_parts_now(&mut self, s: ScopeId, e: ExprId) -> SigParts {
        let db: &Db = &*self.db;
        fn whole(db: &Db, e: ExprId) -> bool {
            let st = |x: ExprId| db.exprs[x].stat != Staticness::None;
            match &db.exprs[e].kind {
                ExprKind::Obj(d) => matches!(
                    &db.decls[*d].kind,
                    DeclKind::Object(o) if o.class == ObjClass::Signal
                        && o.role != ObjRole::ImplicitSignal
                ),
                ExprKind::Index { prefix, indices } => {
                    indices.iter().all(|&i| st(i)) && whole(db, *prefix)
                }
                ExprKind::Slice {
                    prefix,
                    range: yavs_sem::types::RangeC::Explicit { left, right, .. },
                } => st(*left) && st(*right) && whole(db, *prefix),
                ExprKind::Field { prefix, .. } => whole(db, *prefix),
                _ => false,
            }
        }
        if !whole(db, e) {
            return None;
        }
        let l = db.lsp(e)?;
        if !matches!(l.root, ObjRoot::Decl(_)) {
            return None;
        }
        let (o, sels) = self.m.lsp_ref(db, &l, s).ok()?;
        self.m.nets.select(o, &sels).ok()
    }

    /// A process that does nothing unless one edge of one scalar signal happens (or its reset
    /// level holds): the kernel resumes it only then (`yavs_kernel::EdgeTrigger`), where it
    /// would evaluate false conditions and suspend again. Shapes: `process (...) begin if E
    /// then ... end if; end process;` (no `else`), the same with an asynchronous reset,
    /// `if R then ... elsif E then ... end if;` (no `else`), and `process begin ... wait [on
    /// ...] until E; ... end process;` (valid only with one suspension point, checked once
    /// the process is lowered). `E` is `rising_edge(x)` or `falling_edge(x)` (IEEE 1164) or
    /// `x'event and x = '1'` (`'0'`), optionally the first operand of `and`s (the others are
    /// not evaluated on a false edge); `x` is a static element of a signal: `clk`, `v(2)`,
    /// `r.f`. `R` is `r = '1'` (any `std_ulogic` literal) on such an element. The legacy form
    /// and the reset need a `std_ulogic` signal.
    fn edge_trigger(&mut self, s: ScopeId, st: StmtId) -> Option<crate::ProcTrigger> {
        let db: &Db = &*self.db;
        let (sensitivity, body) = match &db.stmts[st].kind {
            StmtKind::Process {
                postponed: false,
                sensitivity,
                body,
                ..
            } => (sensitivity, &body[..]),
            // `q <= d when <edge>;` does nothing on a false edge, as `if <edge> then q <= d; end if`.
            StmtKind::ConcSignalAssign {
                postponed: false,
                guarded: false,
                rhs: Rhs::Conditional(alts),
                ..
            } => {
                let [(_, Some(cond))] = &alts[..] else {
                    return None;
                };
                return self.trigger_of(s, *cond, None, false, None);
            }
            _ => return None,
        };
        // `wait until x = '1'`: the whole condition is the level (its `on` list must be `x`).
        let mut plain = None;
        let (cond, reset, wait) = match (sensitivity, body) {
            (Sensitivity::List(sens), [only]) if !sens.is_empty() => {
                let StmtKind::If {
                    branches,
                    else_: None,
                } = &db.stmts[*only].kind
                else {
                    return None;
                };
                match &branches[..] {
                    [(cond, _)] => (*cond, None, false),
                    [(reset, _), (cond, _)] => (*cond, Some(*reset), false),
                    _ => return None,
                }
            }
            (Sensitivity::None, body) => {
                let mut waits = body.iter().filter_map(|&w| match &db.stmts[w].kind {
                    StmtKind::Wait { on, until, timeout } => {
                        Some((*until, timeout.is_some(), &on[..]))
                    }
                    _ => None,
                });
                let (Some((Some(c), false, on)), None) = (waits.next(), waits.next()) else {
                    return None;
                };
                plain = Some(on.to_vec());
                (c, None, true)
            }
            _ => return None,
        };
        self.trigger_of(s, cond, reset, wait, plain)
    }

    /// The trigger of a clocked process whose edge condition is `cond` (`wait`: the `wait until`
    /// form with the `on` list `plain`), after the shape was recognised by `edge_trigger`.
    fn trigger_of(
        &mut self,
        s: ScopeId,
        cond: ExprId,
        reset: Option<ExprId>,
        wait: bool,
        plain: Option<Vec<Lsp>>,
    ) -> Option<crate::ProcTrigger> {
        let (rising, arg, level) = self.edge_of(cond, plain.is_some())?;
        // The level form (`level` is `arg` itself) resumes on the events of its `on` list
        // (implicit: the signals of the condition), which must be this element alone.
        if level == Some(arg) {
            let mine = self.static_elem(s, arg)?;
            for l in &plain? {
                (self.lsp_elem(s, l)? == mine).then_some(())?;
            }
        }
        let (leaf, elem) = self.static_elem(s, arg)?;
        let legacy = level.is_some();
        if let Some(l) = level {
            // `x'event and x = '1'`: both operands name the same element.
            (self.static_elem(s, l)? == (leaf, elem)).then_some(())?;
        }
        let logic = |e: &Self, l: LeafId| e.m.nets.leaf(l).ty.elem == Elem::Logic;
        let reset = match reset {
            Some(r) => {
                let (x, pos) = self.db.level_of(r)?;
                let (rl, re) = self.static_elem(s, x)?;
                (logic(self, rl) && pos <= 8).then_some((rl, re, pos as u8))
            }
            None => None,
        };
        if (legacy || reset.is_some()) && !logic(self, leaf) {
            return None;
        }
        Some(crate::ProcTrigger {
            rising,
            leaf,
            elem,
            wait,
            legacy,
            reset,
        })
    }

    /// The signal parts the condition of the guard `g` of block `b` reads; `None` when one of
    /// them cannot be resolved here (the `_guard` process then wakes as any process).
    fn guard_reads(&mut self, b: ScopeId, g: DeclId) -> Option<Vec<(LeafId, yavs_kernel::Range)>> {
        let db: &Db = self.db;
        let DeclKind::Object(o) = &db.decls[g].kind else {
            return None;
        };
        let cond = o.default?;
        let mut lsps = Vec::new();
        db.reads(cond, &mut lsps);
        let mut out = Vec::new();
        for l in lsps {
            let (o, sels) = self.m.lsp_ref(db, &l, b).ok()?;
            for (leaf, r) in self.m.nets.select(o, &sels).ok()?.0 {
                if !out.contains(&(leaf, r)) {
                    out.push((leaf, r));
                }
            }
        }
        Some(out)
    }

    /// The signal parts a clocked process (one with an edge trigger) reads, when an activation
    /// of it is a function of those and constants alone (`Db::pure_clocked_reads`); the edge test
    /// itself is not one of them. Shapes: `process (c) begin if <edge> then B end if; end process;`
    /// (also behind a reset branch `if R then A elsif <edge> then B`) and
    /// `process begin wait until <edge>; B end process;`.
    fn idle_reads(&mut self, s: ScopeId, st: StmtId) -> Option<Vec<crate::IdleRead>> {
        let db: &Db = self.db;
        let StmtKind::Process {
            sensitivity,
            body,
            scope,
            ..
        } = &db.stmts[st].kind
        else {
            // `q <= d when <edge>;`
            let StmtKind::ConcSignalAssign {
                rhs: Rhs::Conditional(alts),
                ..
            } = &db.stmts[st].kind
            else {
                return None;
            };
            let [(_, Some(edge))] = &alts[..] else {
                return None;
            };
            let lsps = match self.idle_cache.get(&st) {
                Some(c) => c.clone()?,
                None => {
                    let r = self
                        .edge_extras(*edge)
                        .and_then(|ex| self.db.pure_conc_reads(st, &ex));
                    self.idle_cache.insert(st, r.clone());
                    r?
                }
            };
            return self.idle_parts(s, lsps);
        };
        let (mut conds, bodies, edge): (Vec<ExprId>, Vec<&[StmtId]>, ExprId) =
            match (sensitivity, &body[..]) {
                (Sensitivity::List(_), [only]) => {
                    let StmtKind::If {
                        branches,
                        else_: None,
                    } = &db.stmts[*only].kind
                    else {
                        return None;
                    };
                    match &branches[..] {
                        [(c, b)] => (Vec::new(), vec![&b[..]], *c),
                        [(r, a), (c, b)] => (vec![*r], vec![&a[..], &b[..]], *c),
                        _ => return None,
                    }
                }
                (Sensitivity::None, [w, rest @ ..]) => {
                    let StmtKind::Wait {
                        until: Some(c),
                        timeout: None,
                        ..
                    } = &db.stmts[*w].kind
                    else {
                        return None;
                    };
                    (Vec::new(), vec![rest], *c)
                }
                _ => return None,
            };
        let lsps = match self.idle_cache.get(&st) {
            Some(c) => c.clone()?,
            None => {
                // What the edge test also reads (`rising_edge(c) and en = '1'`).
                let r = self.edge_extras(edge).and_then(|ex| {
                    conds.extend(ex);
                    self.db.pure_clocked_reads(*scope, &conds, &bodies)
                });
                self.idle_cache.insert(st, r.clone());
                r?
            }
        };
        self.idle_parts(s, lsps)
    }

    /// The signal parts a combinational process (`process (a, b) begin ... end process;`, also
    /// `process (all)`) reads, when an activation of it is a function of those and constants
    /// alone (`Db::pure_comb_reads`).
    fn idle_comb_reads(&mut self, s: ScopeId, st: StmtId) -> Option<Vec<crate::IdleRead>> {
        let db: &Db = self.db;
        let StmtKind::Process {
            postponed: false,
            sensitivity: Sensitivity::List(_) | Sensitivity::All,
            body,
            scope,
            ..
        } = &db.stmts[st].kind
        else {
            // `t <= a when c else b;`: the arms its conditions can take.
            let lsps = match self.idle_cache.get(&st) {
                Some(c) => c.clone()?,
                None => {
                    let r = self.db.pure_cond_reads(st);
                    self.idle_cache.insert(st, r.clone());
                    r?
                }
            };
            // Only worth tracking when a read is conditional or left out: with every read
            // unconditional each wake is by a signal the assignment reads.
            self.idle_dead = false;
            let parts = self.idle_parts(s, lsps)?;
            return (self.idle_dead || parts.iter().any(|p| p.guard.is_some())).then_some(parts);
        };
        let lsps = match self.idle_cache.get(&st) {
            Some(c) => c.clone()?,
            None => {
                let r = self.db.pure_comb_reads(*scope, body);
                self.idle_cache.insert(st, r.clone());
                r?
            }
        };
        self.idle_parts(s, lsps)
    }

    /// The kernel signal parts of the reads `reads` of a process in region `s`, with the
    /// conditions under which each is read. A literal that is not an element of a logic signal
    /// with static indices is left out of its conjunction (a weaker condition: the read counts
    /// more often); a conjunction left empty makes the read unconditional.
    fn idle_parts(&mut self, s: ScopeId, reads: Vec<PureRead>) -> Option<Vec<crate::IdleRead>> {
        let mut out: Vec<crate::IdleRead> = Vec::new();
        for r in reads {
            // Conditions of constants and generics hold or not for this instance: a conjunction
            // with a false one is never met, a true one drops out; a read that no conjunction
            // can meet is no read.
            let mut dead = false;
            let r_guard = r.guard.as_ref().map(|alts| {
                let mut live: Vec<Vec<yavs_sem::hir::GuardLit>> = Vec::new();
                for conj in alts {
                    let mut keep = Vec::new();
                    let mut never = false;
                    for l in conj {
                        if l.pos != yavs_sem::hir::GUARD_CONST {
                            keep.push(l.clone());
                            continue;
                        }
                        let v = self
                            .m
                            .eval(self.db, l.sig, Some(s))
                            .ok()
                            .and_then(|v| v.as_int());
                        match v {
                            Some(v) if (v == 1) != l.eq => never = true,
                            _ => {}
                        }
                    }
                    if !never {
                        live.push(keep);
                    }
                }
                dead = live.is_empty();
                live
            });
            if dead {
                self.idle_dead = true;
                continue;
            }
            // Not a read but something impure under a condition that holds here: not pure.
            let Some(lsp) = &r.lsp else {
                return None;
            };
            let guard = r_guard.and_then(|alts| {
                let alts = alts
                    .iter()
                    .map(|conj| {
                        conj.iter()
                            .filter_map(|l| {
                                let (leaf, elem) = self.static_elem(s, l.sig)?;
                                (self.m.nets.leaf(leaf).ty.elem == Elem::Logic).then_some(
                                    crate::IdleLit {
                                        leaf,
                                        elem,
                                        code: u8::try_from(l.pos).ok()?,
                                        eq: l.eq,
                                    },
                                )
                            })
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>();
                (alts.iter().all(|c| !c.is_empty())).then_some(alts)
            });
            let (o, sels) = self.m.lsp_ref(self.db, lsp, s).ok()?;
            for (leaf, range) in self.m.nets.select(o, &sels).ok()?.0 {
                match out.iter_mut().find(|x| (x.leaf, x.range) == (leaf, range)) {
                    // Read twice: under either condition, or unconditionally.
                    Some(x) => {
                        x.guard = match (x.guard.take(), &guard) {
                            (Some(mut a), Some(b)) => {
                                a.extend(b.iter().cloned());
                                Some(a)
                            }
                            _ => None,
                        }
                    }
                    None => out.push(crate::IdleRead {
                        leaf,
                        range,
                        guard: guard.clone(),
                    }),
                }
            }
        }
        Some(out)
    }

    /// The operands of a clock-edge condition besides the edge: none for `rising_edge(c)` and
    /// `c'event and c = '1'` (or the level of a `wait until`), the other operands of an `and`.
    fn edge_extras(&self, cond: ExprId) -> Option<Vec<ExprId>> {
        let db: &Db = &*self.db;
        let ExprKind::Call {
            callee,
            args,
            method_of: None,
            implicit_cond: false,
        } = &db.exprs[cond].kind
        else {
            return None;
        };
        let decl = &db.decls[*callee];
        if !decl.implicit {
            return Some(Vec::new());
        }
        if matches!(decl.name.key().as_str(), "rising_edge" | "falling_edge") {
            return Some(Vec::new());
        }
        let [
            Arg {
                actual: Actual::Expr(a),
                ..
            },
            Arg {
                actual: Actual::Expr(b),
                ..
            },
        ] = &args[..]
        else {
            // The level of a `wait until c = '1'`.
            return self.db.level_of(cond).map(|_| Vec::new());
        };
        if decl.name.key().as_str() == "\"and\"" {
            if db.edge_shape(cond).is_some() {
                return Some(Vec::new());
            }
            if let Some(mut ex) = self.edge_extras(*a) {
                ex.push(*b);
                return Some(ex);
            }
            if self.safe_operand(*a)
                && let Some(mut ex) = self.edge_extras(*b)
            {
                ex.push(*a);
                return Some(ex);
            }
            return None;
        }
        // `c = '1'` of a `wait until`.
        self.db.level_of(cond).map(|_| Vec::new())
    }

    /// A concurrent `x <= not x after T;` (a clock generator, not postponed or guarded, no
    /// rejection limit) on one element of a signal of `std_ulogic`, BIT or BOOLEAN itself (so
    /// no check of the assignment can fail) with the predefined `not`, a delay that is static
    /// and evaluates now to a positive time without calling a subprogram, and the process's
    /// one driver exactly that element. `None` for anything else.
    fn toggle_source(&mut self, s: ScopeId, st: StmtId) -> Option<crate::ProcToggle> {
        let db: &Db = self.db;
        let StmtKind::ConcSignalAssign {
            postponed: false,
            guarded: false,
            target,
            delay: delay @ (Delay::Default | Delay::Transport | Delay::Inertial { reject: None }),
            rhs: Rhs::Simple(Waveform::Elements(elems)),
            ..
        } = &db.stmts[st].kind
        else {
            return None;
        };
        let [
            WaveElem {
                value: Some(e),
                after: Some(after),
            },
        ] = &elems[..]
        else {
            return None;
        };
        let ExprKind::Call {
            callee,
            args,
            method_of: None,
            implicit_cond: false,
        } = &db.exprs[*e].kind
        else {
            return None;
        };
        let [
            Arg {
                actual: Actual::Expr(arg),
                ..
            },
        ] = &args[..]
        else {
            return None;
        };
        let (callee, arg, after, texpr) = (*callee, *arg, *after, target.expr);
        let transport = matches!(delay, Delay::Transport);
        if db.decls[callee].name.key().as_str() != "\"not\""
            || db.exprs[after].stat == Staticness::None
        {
            return None;
        }
        let (leaf, elem) = self.static_elem(s, texpr)?;
        if self.static_elem(s, arg)? != (leaf, elem) {
            return None;
        }
        let (_, shape) = self.static_signal_parts(s, texpr)?;
        let db: &Db = self.db;
        let decl = &db.decls[callee];
        let Shape::Scalar {
            ty,
            range: Some(range),
            elem: kind,
            ..
        } = &shape
        else {
            return None;
        };
        let wk = &db.well_known;
        let ok = match kind {
            // The IEEE `not` of `std_ulogic` (the whole type), or the predefined one of BIT
            // or BOOLEAN (bit-plane logic too).
            Elem::Logic if range.low_high() == (0, 8) => {
                !decl.implicit
                    && db.scopes[decl.scope]
                        .owner
                        .is_some_and(|o| db.decls[o].name.key().as_str() == "std_logic_1164")
            }
            Elem::Logic if range.low_high() == (0, 1) => {
                decl.implicit && (Some(*ty) == wk.boolean || Some(*ty) == wk.bit)
            }
            _ => false,
        };
        if !ok || db.base(db.exprs[arg].subtype) != *ty {
            return None;
        }
        let [dst] = &self.procs.last()?.lsps[..] else {
            return None;
        };
        let [(k, l, r)] = &dst[..] else {
            return None;
        };
        let nets = &self.m.nets;
        if (*l, r.offset, r.len) != (leaf, elem, 1) || nets.drivers[*k].leaf != leaf {
            return None;
        }
        let t = self.static_delay(s, after)?;
        (t > 0).then_some(crate::ProcToggle {
            leaf,
            elem,
            delay: t,
            transport,
        })
    }

    /// The value of the static expression `e`, evaluated now as a probe (undone): `None` when
    /// it needs a subprogram call or fails, so the process keeps it and reports its own errors.
    fn static_val(&mut self, s: ScopeId, e: ExprId) -> Option<Val> {
        let db: &Db = self.db;
        if db.exprs[e].stat == Staticness::None {
            return None;
        }
        let mark = self.m.session.mark();
        let v = self.m.eval(db, e, Some(s));
        let called = self.m.session.mark() != mark;
        self.m.session.undo(db, mark).ok()?;
        v.ok().filter(|_| !called)
    }

    /// The value in femtoseconds of the static `after` expression `e`: `None` when it needs a
    /// subprogram call, fails or is negative.
    fn static_delay(&mut self, s: ScopeId, e: ExprId) -> Option<i64> {
        match self.static_val(s, e)? {
            Val::Phys(t) if t >= 0 => Some(t),
            _ => None,
        }
    }

    /// The value of the static expression `x` of the type and length of `shape` (a scalar or a
    /// one-dimensional array of std_ulogic, BIT or BOOLEAN) in the kernel's codes.
    fn lit_value(&mut self, s: ScopeId, x: ExprId, shape: &Shape) -> Option<yavs_kernel::LogicVec> {
        let (ety, _) = logic_elem(shape)?;
        if self.db.base(self.db.exprs[x].subtype) != shape.ty() {
            return None;
        }
        let v = self.static_val(s, x)?;
        let db: &Db = self.db;
        let vals = match (&v, shape) {
            (Val::Enum(_), Shape::Scalar { .. }) => vec![v.clone()],
            (Val::Array(a), Shape::Array { .. }) if a.bounds.len() == 1 => crate::value::elems(a),
            _ => return None,
        };
        if vals.len() as u64 != shape.len() {
            return None;
        }
        let vals: Vec<&Val> = vals.iter().collect();
        match crate::eval::encode(Elem::Logic, !crate::eval::is_std_ulogic(db, ety), &vals) {
            yavs_kernel::Value::Logic(l) => Some(l),
            _ => None,
        }
    }

    /// The gate of a concurrent `target <= a op b;` or `target <= not a;` (not guarded or
    /// postponed; no delay clause, `transport` or `inertial` without `reject`; one waveform
    /// element, its `after` static as for `toggle_source`): `op` one of `and or xor nand nor xnor`,
    /// the IEEE operator of `std_ulogic` or `std_ulogic_vector` or the predefined one of BIT,
    /// BOOLEAN or their vectors, both operands and the target of the operator's result type,
    /// static signal names of one part each and of one length (so the vector operators' length
    /// check cannot fail), the target's elements of the whole element type (no range check to
    /// make) and one whole driver of the process. `None` for anything else.
    fn gate_source(&mut self, s: ScopeId, st: StmtId) -> Option<crate::ProcGate> {
        let db: &Db = self.db;
        let StmtKind::ConcSignalAssign {
            postponed: false,
            guarded: false,
            target,
            delay: delay @ (Delay::Default | Delay::Transport | Delay::Inertial { reject: None }),
            rhs: Rhs::Simple(Waveform::Elements(elems)),
            ..
        } = &db.stmts[st].kind
        else {
            return None;
        };
        let [
            WaveElem {
                value: Some(e),
                after,
            },
        ] = &elems[..]
        else {
            return None;
        };
        let ExprKind::Call {
            callee,
            args,
            method_of: None,
            implicit_cond: false,
        } = &db.exprs[*e].kind
        else {
            return None;
        };
        use yavs_kernel::Gate;
        let gate = match db.decls[*callee].name.key().as_str() {
            "\"not\"" => Gate::Not,
            "\"and\"" => Gate::And,
            "\"or\"" => Gate::Or,
            "\"xor\"" => Gate::Xor,
            "\"nand\"" => Gate::Nand,
            "\"nor\"" => Gate::Nor,
            "\"xnor\"" => Gate::Xnor,
            "\"=\"" => Gate::Eq,
            "\"/=\"" => Gate::Ne,
            _ => return None,
        };
        let rel = matches!(gate, Gate::Eq | Gate::Ne);
        let ops: Vec<ExprId> = args
            .iter()
            .map(|a| match a.actual {
                Actual::Expr(x) => Some(x),
                _ => None,
            })
            .collect::<Option<_>>()?;
        if ops.len() != if gate == Gate::Not { 1 } else { 2 } {
            return None;
        }
        let (callee, e, after, texpr) = (*callee, *e, *after, target.expr);
        let transport = matches!(delay, Delay::Transport);
        // The operands are of one type, and so are the result and the target (not for `=`).
        let ty = db.base(db.exprs[ops[0]].subtype);
        if ops.iter().any(|&x| db.base(db.exprs[x].subtype) != ty)
            || (!rel && db.base(db.exprs[e].subtype) != ty)
        {
            return None;
        }
        let (dparts, tshape) = self.static_signal_parts(s, texpr)?;
        // What the operands must look like: the target, or for `=` the signal operand.
        let (srcs, rshape) = self.operands(s, &ops, (!rel).then_some(&tshape))?;
        let db: &Db = self.db;
        let (ety, range) = logic_elem(&tshape)?;
        let decl = &db.decls[callee];
        let wk = &db.well_known;
        let ok = if rel {
            // The predefined `=` of BIT, BOOLEAN and std_ulogic (codes compare) and their
            // vectors; BOOLEAN result in a scalar target.
            decl.implicit
                && Some(ety) == wk.boolean
                && range.low_high() == (0, 1)
                && matches!(tshape, Shape::Scalar { .. })
        } else {
            match range.low_high() {
                // The IEEE operators of `std_ulogic` (the whole type) and its vectors.
                (0, 8) => {
                    !decl.implicit
                        && db.scopes[decl.scope]
                            .owner
                            .is_some_and(|o| db.decls[o].name.key().as_str() == "std_logic_1164")
                }
                // The predefined ones of BIT and BOOLEAN (bit-plane logic too).
                (0, 1) => decl.implicit && (Some(ety) == wk.boolean || Some(ety) == wk.bit),
                _ => false,
            }
        };
        let [dpart] = dparts[..] else {
            return None;
        };
        let [dst] = &self.procs.last()?.lsps[..] else {
            return None;
        };
        let [(k, dl, dr)] = dst[..] else {
            return None;
        };
        let nets = &self.m.nets;
        if !ok || dpart != (dl, dr) || nets.drivers[k].leaf != dl || nets.drivers[k].range != dr {
            return None;
        }
        let delay = match after {
            None => 0,
            Some(a) => self.static_delay(s, a)?,
        };
        Some(crate::ProcGate {
            gate,
            a: srcs[0].clone(),
            b: srcs.last()?.clone(),
            len: if rel { rshape.len() as u32 } else { dr.len },
            delay,
            transport,
        })
    }

    /// The operands `ops` of a gate or a relation as sources: static signal names of one part
    /// each and the shape `want` (the first signal operand's when `None`), or static values of
    /// that shape; at least one is a signal. Also the shape.
    fn operands(
        &mut self,
        s: ScopeId,
        ops: &[ExprId],
        want: Option<&Shape>,
    ) -> Option<(Vec<crate::ProcSrc>, Shape)> {
        let names: Vec<_> = ops
            .iter()
            .map(|&x| self.static_signal_parts(s, x))
            .collect();
        let rshape = match want {
            Some(w) => w.clone(),
            None => names.iter().flatten().next()?.1.clone(),
        };
        logic_elem(&rshape)?;
        let mut srcs = Vec::with_capacity(2);
        for (&x, n) in ops.iter().zip(names) {
            srcs.push(match n {
                Some((p, shape)) => {
                    let [(l, r)] = p[..] else {
                        return None;
                    };
                    if shape.ty() != rshape.ty()
                        || shape.len() != rshape.len()
                        || self.m.nets.leaf(l).ty.elem != Elem::Logic
                    {
                        return None;
                    }
                    crate::ProcSrc::Sig(l, r)
                }
                None => crate::ProcSrc::Lit(self.lit_value(s, x, &rshape)?),
            });
        }
        if srcs.iter().all(|x| matches!(x, crate::ProcSrc::Lit(_))) {
            return None;
        }
        Some((srcs, rshape))
    }

    /// The mux of a concurrent `target <= a when c else b;` (not guarded or postponed; no delay
    /// clause, `transport` or `inertial` without `reject`; two waveform elements of one value
    /// each, their `after` static): `c` the test of one logic element against a set of codes
    /// (`x = '1'`, `x /= '0'`, `'1' = x`, `?? x` of std_ulogic, a BOOLEAN name), each arm a copy
    /// of a static signal name as `copy_source` takes one, or a static value of the
    /// target's type and length when the target is one part of std_ulogic, BIT or BOOLEAN
    /// elements of the whole type. `None` for anything else.
    fn mux_source(&mut self, s: ScopeId, st: StmtId) -> Option<crate::ProcMux> {
        let db: &Db = self.db;
        let StmtKind::ConcSignalAssign {
            postponed: false,
            guarded: false,
            target,
            delay: delay @ (Delay::Default | Delay::Transport | Delay::Inertial { reject: None }),
            rhs: Rhs::Conditional(alts),
            ..
        } = &db.stmts[st].kind
        else {
            return None;
        };
        let [
            (Waveform::Elements(w1), Some(c)),
            (Waveform::Elements(w2), None),
        ] = &alts[..]
        else {
            return None;
        };
        let (
            [
                WaveElem {
                    value: Some(e1),
                    after: a1,
                },
            ],
            [
                WaveElem {
                    value: Some(e2),
                    after: a2,
                },
            ],
        ) = (&w1[..], &w2[..])
        else {
            return None;
        };
        let (c, texpr) = (*c, target.expr);
        let (e1, e2, a1, a2) = (*e1, *e2, *a1, *a2);
        let transport = matches!(delay, Delay::Transport);
        let cond = self.mux_cond(s, c)?;
        let (_, tshape) = self.static_signal_parts(s, texpr)?;
        let mut srcs = [Vec::new(), Vec::new()];
        let mut delays = [0; 2];
        // [else, then]
        for (i, (e, after)) in [(e2, a2), (e1, a1)].into_iter().enumerate() {
            srcs[i] = match self.copy_parts(s, e, texpr) {
                Some(v) => (v.into_iter())
                    .map(|(l, r)| crate::ProcSrc::Sig(l, r))
                    .collect(),
                None => {
                    // A value: the target is one whole driver of whole-type logic elements.
                    let db: &Db = self.db;
                    let (ety, range) = logic_elem(&tshape)?;
                    let whole = if crate::eval::is_std_ulogic(db, ety) {
                        (0, 8)
                    } else {
                        (0, 1)
                    };
                    let [dst] = &self.procs.last()?.lsps[..] else {
                        return None;
                    };
                    let [(k, dl, dr)] = dst[..] else {
                        return None;
                    };
                    let nets = &self.m.nets;
                    if range.low_high() != whole
                        || nets.drivers[k].leaf != dl
                        || nets.drivers[k].range != dr
                    {
                        return None;
                    }
                    vec![crate::ProcSrc::Lit(self.lit_value(s, e, &tshape)?)]
                }
            };
            delays[i] = match after {
                None => 0,
                Some(a) => self.static_delay(s, a)?,
            };
        }
        Some(crate::ProcMux {
            cond,
            srcs,
            delay: delays,
            transport,
        })
    }

    /// The assignment `target <= v1 when c1 else v2 when c2 else ... else vn;` or `target <= v;`
    /// (not guarded or postponed; no delay clause, `transport` or `inertial` without `reject`;
    /// one waveform element of one value per arm, its `after` static) as a [`crate::ProcProg`]:
    /// each condition a tree of BOOLEAN operators over relations of logic operands, tests of
    /// one logic element, BOOLEAN names and static values (`prog_node`); each value a copy as
    /// `copy_parts` takes one, or a tree of logic operators over static signal names and static
    /// values when the target is one whole driver of whole-type logic elements. `None` for
    /// anything else, and for a statement that is no more than a copy or a literal.
    fn prog_source(&mut self, s: ScopeId, st: StmtId) -> Option<crate::ProcProg> {
        let (arms_in, texpr, transport) = self.prog_arms(st)?;
        // The same names are looked up several times (shape, test, operand): once per expression.
        self.parts_memo = Some(HashMap::new());
        let r = self
            .static_signal_parts(s, texpr)
            .and_then(|(_, tshape)| self.prog_build(s, arms_in, texpr, tshape, transport));
        self.parts_memo = None;
        r
    }

    /// The arms of a concurrent assignment `prog_source` takes: condition, value and `after` of
    /// each, the target and whether it is `transport`.
    #[allow(clippy::type_complexity)]
    fn prog_arms(
        &self,
        st: StmtId,
    ) -> Option<(Vec<(Option<ExprId>, ExprId, Option<ExprId>)>, ExprId, bool)> {
        let db: &Db = self.db;
        let StmtKind::ConcSignalAssign {
            postponed: false,
            guarded: false,
            target,
            delay: delay @ (Delay::Default | Delay::Transport | Delay::Inertial { reject: None }),
            rhs,
            ..
        } = &db.stmts[st].kind
        else {
            return None;
        };
        let alts: Vec<(&Waveform, Option<ExprId>)> = match rhs {
            Rhs::Simple(w) => vec![(w, None)],
            Rhs::Conditional(a) => a.iter().map(|(w, c)| (w, *c)).collect(),
            Rhs::Selected { .. } => return None,
        };
        let last = alts.len() - 1;
        if alts
            .iter()
            .enumerate()
            .any(|(i, a)| a.1.is_none() != (i == last))
        {
            return None;
        }
        // (condition, value, after) per arm, copied out of the database.
        let mut arms_in = Vec::with_capacity(alts.len());
        for (w, c) in alts {
            let Waveform::Elements(w) = w else {
                return None;
            };
            let [
                WaveElem {
                    value: Some(e),
                    after,
                },
            ] = &w[..]
            else {
                return None;
            };
            if c.is_some_and(|c| !self.prog_shape(c)) || !self.prog_shape(*e) {
                return None;
            }
            arms_in.push((c, *e, *after));
        }
        Some((arms_in, target.expr, matches!(delay, Delay::Transport)))
    }

    fn prog_build(
        &mut self,
        s: ScopeId,
        arms_in: Vec<(Option<ExprId>, ExprId, Option<ExprId>)>,
        texpr: ExprId,
        tshape: Shape,
        transport: bool,
    ) -> Option<crate::ProcProg> {
        let mut nodes = Vec::new();
        let mut arms = Vec::with_capacity(arms_in.len());
        for (c, e, after) in arms_in {
            let n0 = nodes.len() as u32;
            let cond = match c {
                Some(c) => Some(self.prog_cond(s, c, &mut nodes)?),
                None => None,
            };
            let n1 = nodes.len() as u32;
            let vals = match self.copy_parts(s, e, texpr) {
                Some(v) => (v.into_iter())
                    .map(|(l, r)| crate::ProcOpnd::Src(crate::ProcSrc::Sig(l, r)))
                    .collect(),
                None => {
                    if !self.whole_logic_target(&tshape) {
                        return None;
                    }
                    let (o, shape) = self.prog_node(s, e, Some(&tshape), &mut nodes)?;
                    if shape.ty() != tshape.ty() || shape.len() != tshape.len() {
                        return None;
                    }
                    vec![o]
                }
            };
            let n2 = nodes.len() as u32;
            let delay = match after {
                None => 0,
                Some(a) => self.static_delay(s, a)?,
            };
            arms.push(crate::ProcArm {
                cond,
                cond_nodes: n0..n1,
                vals,
                val_nodes: n1..n2,
                delay,
            });
        }
        if nodes.is_empty() && arms.len() == 1 {
            return None;
        }
        Some(crate::ProcProg {
            nodes,
            arms,
            transport,
        })
    }

    /// A static expression that `static_val` evaluates without a subprogram call: literals,
    /// objects, selections and the predefined operators over such. (A call to a declared
    /// function is evaluated, and undone, only to find it out; in a cold world that lowers
    /// the function.)
    fn static_cheap(&self, e: ExprId) -> bool {
        let db: &Db = self.db;
        match &db.exprs[e].kind {
            ExprKind::Const(_) | ExprKind::Obj(_) => true,
            ExprKind::Index { prefix, indices } => {
                self.static_cheap(*prefix) && indices.iter().all(|&i| self.static_cheap(i))
            }
            ExprKind::Slice { prefix, .. } | ExprKind::Field { prefix, .. } => {
                self.static_cheap(*prefix)
            }
            ExprKind::Qualified { operand, .. }
            | ExprKind::Conv { operand, .. }
            | ExprKind::ImplicitConv { operand } => self.static_cheap(*operand),
            ExprKind::Aggregate { assocs, .. } => assocs.iter().all(|a| self.static_cheap(a.value)),
            ExprKind::Call {
                callee,
                args,
                method_of: None,
                ..
            } => {
                db.decls[*callee].implicit
                    && args.iter().all(|a| match a.actual {
                        Actual::Expr(x) => self.static_cheap(x),
                        _ => false,
                    })
            }
            _ => false,
        }
    }

    /// Whether `e` is made only of what `prog_node` can take, judged by the kinds of the
    /// expressions alone (no evaluation): static values, signal names, and the operators
    /// `prog_node` knows over such. It keeps the statements that cannot be programs from costing
    /// a probe each at elaboration.
    fn prog_shape(&self, e: ExprId) -> bool {
        let db: &Db = self.db;
        if db.exprs[e].stat != Staticness::None {
            return self.static_cheap(e);
        }
        match &db.exprs[e].kind {
            ExprKind::Obj(_)
            | ExprKind::Index { .. }
            | ExprKind::Slice { .. }
            | ExprKind::Field { .. } => true,
            ExprKind::Call {
                callee,
                args,
                method_of: None,
                ..
            } => {
                matches!(
                    db.decls[*callee].name.key().as_str(),
                    "\"not\""
                        | "\"and\""
                        | "\"or\""
                        | "\"xor\""
                        | "\"nand\""
                        | "\"nor\""
                        | "\"xnor\""
                        | "\"=\""
                        | "\"/=\""
                        | "\"??\""
                ) && args.iter().all(|a| match a.actual {
                    Actual::Expr(x) => self.prog_shape(x),
                    _ => false,
                })
            }
            _ => false,
        }
    }

    /// The target is one whole driver of the process, of logic elements of the whole type
    /// (std_ulogic, BIT or BOOLEAN): nothing a value assigned to it can be checked against.
    fn whole_logic_target(&mut self, tshape: &Shape) -> bool {
        let db: &Db = self.db;
        let Some((ety, range)) = logic_elem(tshape) else {
            return false;
        };
        let whole = if crate::eval::is_std_ulogic(db, ety) {
            (0, 8)
        } else {
            (0, 1)
        };
        let Some([dst]) = self.procs.last().map(|p| &p.lsps[..]) else {
            return false;
        };
        let [(k, dl, dr)] = dst[..] else {
            return false;
        };
        let nets = &self.m.nets;
        range.low_high() == whole && nets.drivers[k].leaf == dl && nets.drivers[k].range == dr
    }

    /// The condition `c` as an operand: a BOOLEAN element.
    fn prog_cond(
        &mut self,
        s: ScopeId,
        c: ExprId,
        nodes: &mut Vec<crate::ProcNode>,
    ) -> Option<crate::ProcOpnd> {
        let (o, shape) = self.prog_node(s, c, None, nodes)?;
        let boolean = Some(shape.ty()) == self.db.well_known.boolean;
        (boolean && matches!(shape, Shape::Scalar { .. })).then_some(o)
    }

    /// The shape of a scalar expression of an enumeration type (no evaluation).
    fn scalar_shape(&mut self, s: ScopeId, e: ExprId) -> Option<Shape> {
        let db: &Db = self.db;
        let st = db.exprs[e].subtype;
        if !db.types[db.base(st)].kind.is_scalar() {
            return None;
        }
        self.m.shape(db, st, Some(s), db.exprs[e].span).ok()?
    }

    /// The shape of the operand `x` of an operator: that of its first static signal name, found
    /// through logical operators, or of a scalar expression that reads a signal; `None` for a
    /// static value or anything else.
    fn tree_shape(&mut self, s: ScopeId, x: ExprId) -> Option<Shape> {
        if let Some((_, sh)) = self.static_signal_parts(s, x) {
            return Some(sh);
        }
        if self.db.exprs[x].stat != Staticness::None {
            return None;
        }
        if let ExprKind::Call { callee, args, .. } = &self.db.exprs[x].kind
            && matches!(
                self.db.decls[*callee].name.key().as_str(),
                "\"not\"" | "\"and\"" | "\"or\"" | "\"xor\"" | "\"nand\"" | "\"nor\"" | "\"xnor\""
            )
        {
            let args: Vec<ExprId> = args
                .iter()
                .filter_map(|a| match a.actual {
                    Actual::Expr(x) => Some(x),
                    _ => None,
                })
                .collect();
            return args.into_iter().find_map(|a| self.tree_shape(s, a));
        }
        self.scalar_shape(s, x)
    }

    /// A test of one element of a signal of an enumeration or integer type against a static
    /// value, `exec.state = S_IDLE` or `n /= 3` (the predefined `=` or `/=`, either side): the
    /// element, the bytes of the value as the signal holds them, and whether it is `/=`. `None`
    /// when the value does not fit the element (it is then never equal, which the process
    /// reports itself).
    fn bytes_cond(&mut self, s: ScopeId, e: ExprId) -> Option<(LeafId, Range, Vec<u8>, bool)> {
        let ExprKind::Call {
            callee,
            args,
            method_of: None,
            implicit_cond: false,
        } = &self.db.exprs[e].kind
        else {
            return None;
        };
        let decl = &self.db.decls[*callee];
        let ne = match decl.name.key().as_str() {
            "\"=\"" if decl.implicit => false,
            "\"/=\"" if decl.implicit => true,
            _ => return None,
        };
        let [
            Arg {
                actual: Actual::Expr(l),
                ..
            },
            Arg {
                actual: Actual::Expr(r),
                ..
            },
        ] = &args[..]
        else {
            return None;
        };
        for (sig, val) in [(*l, *r), (*r, *l)] {
            let Some((p, Shape::Scalar { .. })) = self.static_signal_parts(s, sig) else {
                continue;
            };
            let [(leaf, range)] = p[..] else {
                continue;
            };
            let Elem::Bytes(w) = self.m.nets.leaf(leaf).ty.elem else {
                continue;
            };
            let n = match self.static_val(s, val)? {
                Val::Int(n) => n,
                Val::Enum(p) => i64::from(p),
                _ => return None,
            };
            let fits = match w {
                8 => true,
                4 => (-(1i64 << 31)..1i64 << 32).contains(&n),
                _ => (0..1i64 << (8 * w)).contains(&n),
            };
            if range.len != 1 || !fits {
                return None;
            }
            return Some((
                leaf,
                range,
                (n as u64).to_le_bytes()[..w as usize].to_vec(),
                ne,
            ));
        }
        None
    }

    /// The expression `e` as an operand, appending the nodes it needs to `nodes`, and its shape:
    /// a static signal name of one part (`want`, if given, is its shape), a static value (of
    /// the shape `want`, or a scalar's own), a test of one logic element, a relation `=` or `/=`
    /// of the predefined kind between operands of one logic shape, or a logical operator over
    /// such operands (all of one type and length, so none can fail); a call to anything else
    /// is `None`. At least one operand of an operator is not a value.
    fn prog_node(
        &mut self,
        s: ScopeId,
        e: ExprId,
        want: Option<&Shape>,
        nodes: &mut Vec<crate::ProcNode>,
    ) -> Option<(crate::ProcOpnd, Shape)> {
        use crate::{ProcNode, ProcOp, ProcOpnd, ProcSrc};
        if let Some((p, shape)) = self.static_signal_parts(s, e) {
            let [(l, r)] = p[..] else {
                return None;
            };
            if self.m.nets.leaf(l).ty.elem != Elem::Logic
                || logic_elem(&shape).is_none()
                || want.is_some_and(|w| w.ty() != shape.ty() || w.len() != shape.len())
            {
                return None;
            }
            return Some((ProcOpnd::Src(ProcSrc::Sig(l, r)), shape));
        }
        if self.db.exprs[e].stat != Staticness::None {
            let shape = match want {
                Some(w) => w.clone(),
                None => self.scalar_shape(s, e)?,
            };
            logic_elem(&shape)?;
            let v = self.lit_value(s, e, &shape)?;
            return Some((ProcOpnd::Src(ProcSrc::Lit(v)), shape));
        }
        let ExprKind::Call {
            callee,
            args,
            method_of: None,
            ..
        } = &self.db.exprs[e].kind
        else {
            return None;
        };
        let callee = *callee;
        let ops: Vec<ExprId> = args
            .iter()
            .map(|a| match a.actual {
                Actual::Expr(x) => Some(x),
                _ => None,
            })
            .collect::<Option<_>>()?;
        if let Some(crate::ProcCond::Mask((l, r), mask)) = self.mask_cond(s, e) {
            let shape = self.scalar_shape(s, e)?;
            nodes.push(ProcNode {
                op: ProcOp::Mask(l, r, mask),
                len: 1,
            });
            return Some((ProcOpnd::Node(nodes.len() as u32 - 1), shape));
        }
        if let Some((l, r, bytes, ne)) = self.bytes_cond(s, e) {
            let shape = self.scalar_shape(s, e)?;
            nodes.push(ProcNode {
                op: ProcOp::Bytes(l, r, bytes, ne),
                len: 1,
            });
            return Some((ProcOpnd::Node(nodes.len() as u32 - 1), shape));
        }
        use yavs_kernel::Gate;
        let decl = &self.db.decls[callee];
        let (implicit, decl_scope) = (decl.implicit, decl.scope);
        let gate = match self.db.decls[callee].name.key().as_str() {
            "\"not\"" => Gate::Not,
            "\"and\"" => Gate::And,
            "\"or\"" => Gate::Or,
            "\"xor\"" => Gate::Xor,
            "\"nand\"" => Gate::Nand,
            "\"nor\"" => Gate::Nor,
            "\"xnor\"" => Gate::Xnor,
            "\"=\"" => Gate::Eq,
            "\"/=\"" => Gate::Ne,
            _ => return None,
        };
        let rel = matches!(gate, Gate::Eq | Gate::Ne);
        if ops.len() != if gate == Gate::Not { 1 } else { 2 } {
            return None;
        }
        // The shape of the operands: the wanted one for a logical operator, else the first
        // operand's that is a signal (or a BOOLEAN expression).
        let mut rshape = if rel { None } else { want.cloned() };
        for &x in &ops {
            if rshape.is_some() {
                break;
            }
            rshape = self.tree_shape(s, x);
        }
        let rshape = rshape?;
        let (ety, range) = logic_elem(&rshape)?;
        let ty = rshape.ty();
        if ops
            .iter()
            .any(|&x| self.db.base(self.db.exprs[x].subtype) != ty)
            || (!rel && self.db.base(self.db.exprs[e].subtype) != ty)
        {
            return None;
        }
        let wk = &self.db.well_known;
        let ok = if rel {
            implicit
        } else {
            match range.low_high() {
                (0, 8) => {
                    !implicit
                        && self.db.scopes[decl_scope].owner.is_some_and(|o| {
                            self.db.decls[o].name.key().as_str() == "std_logic_1164"
                        })
                }
                (0, 1) => implicit && (Some(ety) == wk.boolean || Some(ety) == wk.bit),
                _ => false,
            }
        };
        if !ok {
            return None;
        }
        let mut opnds = Vec::with_capacity(2);
        for &x in &ops {
            let (o, sh) = self.prog_node(s, x, Some(&rshape), nodes)?;
            if sh.ty() != ty || sh.len() != rshape.len() {
                return None;
            }
            opnds.push(o);
        }
        if opnds
            .iter()
            .all(|o| matches!(o, ProcOpnd::Src(ProcSrc::Lit(_))))
        {
            return None;
        }
        let b = opnds.last()?.clone();
        nodes.push(ProcNode {
            op: ProcOp::Gate(gate, opnds[0].clone(), b),
            len: rshape.len() as u32,
        });
        let shape = if rel {
            self.scalar_shape(s, e)?
        } else {
            rshape
        };
        Some((ProcOpnd::Node(nodes.len() as u32 - 1), shape))
    }

    /// What a condition tests: `x = '1'` (or `/=`, either side, BIT and BOOLEAN with the codes of
    /// '0' and '1'), `?? x` of std_ulogic ('1' and 'H') or a BOOLEAN name, all on a static
    /// element `x` of a logic signal; or the predefined `a = b` or `a /= b` of logic signals and
    /// static values of one type and length.
    fn mux_cond(&mut self, s: ScopeId, c: ExprId) -> Option<crate::ProcCond> {
        self.mask_cond(s, c).or_else(|| self.rel_cond(s, c))
    }

    fn rel_cond(&mut self, s: ScopeId, c: ExprId) -> Option<crate::ProcCond> {
        let db: &Db = self.db;
        let ExprKind::Call {
            callee,
            args,
            method_of: None,
            implicit_cond: false,
        } = &db.exprs[c].kind
        else {
            return None;
        };
        let decl = &db.decls[*callee];
        let ne = match decl.name.key().as_str() {
            "\"=\"" if decl.implicit => false,
            "\"/=\"" if decl.implicit => true,
            _ => return None,
        };
        let [
            Arg {
                actual: Actual::Expr(l),
                ..
            },
            Arg {
                actual: Actual::Expr(r),
                ..
            },
        ] = &args[..]
        else {
            return None;
        };
        let (mut srcs, shape) = self.operands(s, &[*l, *r], None)?;
        let (b, a) = (srcs.pop()?, srcs.pop()?);
        Some(crate::ProcCond::Eq {
            a,
            b,
            len: shape.len() as u32,
            ne,
        })
    }

    fn mask_cond(&mut self, s: ScopeId, c: ExprId) -> Option<crate::ProcCond> {
        let db: &Db = self.db;
        let (x, mask) = if let Some((x, code, eq)) = db.level_cmp(c) {
            // An enumeration position of 16 or more is no logic code (the element is checked below).
            let m = 1u16.checked_shl(code)?;
            (x, if eq { m } else { !m & 0x1ff })
        } else if let ExprKind::Call {
            callee,
            args,
            method_of: None,
            ..
        } = &db.exprs[c].kind
            && db.decls[*callee].name.key().as_str() == "\"??\""
            && db.scopes[db.decls[*callee].scope]
                .owner
                .is_some_and(|o| db.decls[o].name.key().as_str() == "std_logic_1164")
            && let [
                Arg {
                    actual: Actual::Expr(x),
                    ..
                },
            ] = &args[..]
        {
            (*x, 1 << 3 | 1 << 7)
        } else if Some(db.base(db.exprs[c].subtype)) == db.well_known.boolean {
            (c, 1 << 3)
        } else {
            return None;
        };
        let (p, shape) = self.static_signal_parts(s, x)?;
        let [(l, r)] = p[..] else {
            return None;
        };
        let ok = r.len == 1
            && matches!(shape, Shape::Scalar { .. })
            && self.m.nets.leaf(l).ty.elem == Elem::Logic;
        ok.then_some(crate::ProcCond::Mask((l, r), mask))
    }

    /// The leaf and element a static signal name denotes.
    fn static_elem(&mut self, s: ScopeId, e: ExprId) -> Option<(LeafId, u32)> {
        let db: &Db = &*self.db;
        let (d, path) = self.static_signal(e)?;
        let l = Lsp {
            root: ObjRoot::Decl(d),
            path: path.into(),
            stat: Staticness::Globally,
            span: db.exprs[e].span,
        };
        self.lsp_elem(s, &l)
    }

    /// The leaf and element a static longest prefix denotes.
    fn lsp_elem(&mut self, s: ScopeId, l: &Lsp) -> Option<(LeafId, u32)> {
        let db: &Db = &*self.db;
        // A probe: the same name is resolved for real when the process is lowered.
        let (o, sels) = self.m.lsp_ref(db, l, s).ok()?;
        match self.m.nets.select(o, &sels).ok()?.0[..] {
            [(leaf, r)] if r.len == 1 => Some((leaf, r.offset)),
            _ => None,
        }
    }

    /// `rising_edge(x)` or `falling_edge(x)` of IEEE 1164, `x'event and x = '1'` (or `'0'`;
    /// either order, `Db::edge_shape`), or an `and` of BOOLEANs whose first operand is one:
    /// whether it is rising, the argument and, for the legacy form, the other mention of the
    /// signal.
    fn edge_of(&self, cond: ExprId, plain: bool) -> Option<(bool, ExprId, Option<ExprId>)> {
        let db: &Db = &*self.db;
        // `wait until x = '1'`: it resumes on an event on `x` that ends on '1', as the legacy form.
        if plain && let Some((x, pos @ (2 | 3))) = db.level_of(cond) {
            return Some((pos == 3, x, Some(x)));
        }
        if let Some(s) = db.edge_shape(cond) {
            return Some((s.rising, s.sig, s.other));
        }
        let ExprKind::Call {
            callee,
            args,
            method_of: None,
            implicit_cond: false,
        } = &db.exprs[cond].kind
        else {
            return None;
        };
        // The predefined `and` of BOOLEAN (it does not evaluate its second operand after a false
        // first one).
        let decl = &db.decls[*callee];
        let boolean = Some(db.base(db.exprs[cond].subtype)) == db.well_known.boolean;
        if !(decl.implicit && boolean && args.len() == 2 && decl.name.key().as_str() == "\"and\"") {
            return None;
        }
        let (Actual::Expr(first), Actual::Expr(second)) = (&args[0].actual, &args[1].actual) else {
            return None;
        };
        if let Some(x) = self.edge_of(*first, false) {
            return Some(x);
        }
        // The edge as a later operand: the earlier ones cannot fail (a level of a static
        // signal name), so skipping them on a false edge changes nothing.
        if self.safe_operand(*first) {
            return self.edge_of(*second, false);
        }
        None
    }

    /// A BOOLEAN operand whose evaluation cannot fail or have an effect: `x = '1'` on a static
    /// signal name, or an `and` of such.
    fn safe_operand(&self, e: ExprId) -> bool {
        let db: &Db = &*self.db;
        if let Some((x, _)) = self.db.level_of(e) {
            return self.static_signal(x).is_some();
        }
        match &db.exprs[e].kind {
            ExprKind::Call {
                callee,
                args,
                method_of: None,
                implicit_cond: false,
            } if db.decls[*callee].implicit
                && db.decls[*callee].name.key().as_str() == "\"and\""
                && args.len() == 2 =>
            {
                args.iter().all(|a| match a.actual {
                    Actual::Expr(x) => self.safe_operand(x),
                    _ => false,
                })
            }
            _ => false,
        }
    }

    /// A name made of a signal, static indices and fields: the signal and the steps.
    fn static_signal(&self, e: ExprId) -> Option<(DeclId, Vec<LspStep>)> {
        let db: &Db = &*self.db;
        match &db.exprs[e].kind {
            ExprKind::Obj(d) => Some((*d, Vec::new())),
            ExprKind::Index { prefix, indices } => {
                if indices
                    .iter()
                    .any(|&i| db.exprs[i].stat == Staticness::None)
                {
                    return None;
                }
                let (d, mut p) = self.static_signal(*prefix)?;
                p.push(LspStep::Index(indices.clone()));
                Some((d, p))
            }
            ExprKind::Field { prefix, elem } => {
                let (d, mut p) = self.static_signal(*prefix)?;
                p.push(LspStep::Field(*elem));
                Some((d, p))
            }
            _ => None,
        }
    }

    /// Bind what [`Elab::pending`] and [`Elab::deferred`] hold, in elaboration order.
    fn bind_late(&mut self) -> R<()> {
        for (s, h) in std::mem::take(&mut self.pending) {
            let db: &Db = self.db;
            let x = db
                .external_alias(h)
                .ok_or_else(|| Box::new(ice(db.decls[h].span, "not an external alias")))?;
            self.m.bind(db, s, h, x)?;
        }
        // Those of package subprograms: one that cannot be bound is an error when the
        // subprogram is called (LRM 8.7: evaluated when the call evaluates it), not here.
        for (u, h) in std::mem::take(&mut self.pkg_uses) {
            let db: &Db = self.db;
            let x = db
                .external_alias(h)
                .ok_or_else(|| Box::new(ice(db.decls[h].span, "not an external alias")))?;
            let ps = self.m.pkg_scope(db, u);
            match self.m.bind(db, ps, h, x) {
                Ok(()) => {
                    self.m.pkg_ext.insert(h, ps);
                }
                Err(d) if d.code == codes::EXTERNAL_NOT_FOUND => {
                    self.m.ext_unbound.insert(h, d.message.clone());
                }
                Err(d) => return Err(d),
            }
        }
        for (idx, s, lsps) in std::mem::take(&mut self.deferred) {
            let mut refs = Vec::with_capacity(lsps.len());
            for l in &lsps {
                let (o, sels) = self.m.lsp_ref(self.db, l, s)?;
                refs.push((o, sels, l.span));
            }
            self.set_drivers(idx, refs)?;
        }
        Ok(())
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
        let idx = self.add_process(s, st, name, postponed);
        self.set_drivers(idx, refs)
    }

    /// A process instance without drivers yet.
    fn add_process(&mut self, s: ScopeId, st: StmtId, name: Ident, postponed: bool) -> ProcIdx {
        let ps = self
            .m
            .hier
            .add(Some(s), ScopeKind::Process { stmt: st }, name);
        self.procs.push(ProcInst {
            scope: ps,
            stmt: st,
            postponed,
            drivers: Vec::new(),
            lsps: Vec::new(),
            actual: None,
            trigger: None,
            copy: None,
            toggle: None,
            gate: None,
            mux: None,
            prog: None,
            idle: None,
            guard: None,
        });
        ProcIdx(self.procs.len() as u32 - 1)
    }

    /// The drivers of process `idx` for the given static names.
    fn set_drivers(&mut self, idx: ProcIdx, refs: Vec<(ObjId, Vec<Sel>, Span)>) -> R<()> {
        // Merge the driven ranges per leaf: a process has one driver per scalar subelement.
        let mut per_lsp = Vec::with_capacity(refs.len());
        let mut by_leaf: Vec<(LeafId, Range)> = Vec::new();
        for (o, sels, span) in refs {
            let (rs, _) = self.m.select(o, &sels, span)?;
            by_leaf.extend(rs.iter().copied());
            per_lsp.push(rs);
        }
        // LRM 14.7.2: a process driving a subelement of a composite resolved as a whole has a
        // driver for every scalar subelement of it, so the kernel sees whole composite sources.
        let mut extra = Vec::new();
        for &(l, r) in &by_leaf {
            if !matches!(self.m.nets.leaf(l).resolution, Some(LeafRes::Whole(_))) {
                continue;
            }
            let Some(w) = self.m.nets.whole_of(l) else {
                continue;
            };
            // ponytail: linear in the instances of the composite; compute the element index if
            // large arrays of resolved composites get many drivers.
            for parts in &w.instances {
                if parts.iter().any(|&(pl, pr)| pl == l && pr.overlaps(r)) {
                    extra.extend(parts.iter().copied());
                }
            }
        }
        by_leaf.extend(extra);
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
                        // `merged` is sorted and disjoint: the last range starting at or
                        // before `r` is the one that holds it.
                        let k = merged
                            .partition_point(|(ml, mr)| (*ml, mr.offset) <= (l, r.offset))
                            .checked_sub(1)
                            .filter(|&k| merged[k].0 == l && merged[k].1.contains(r))
                            .expect("merged ranges cover every lsp range");
                        (base + k, l, r)
                    })
                    .collect()
            })
            .collect();
        let p = &mut self.procs[idx.index()];
        p.drivers = drivers;
        p.lsps = lsps;
        Ok(())
    }

    /// LRM 14.7.2: an unresolved signal has at most one source per element (process drivers
    /// and `out`/`inout`/`buffer` ports).
    fn check_sources(&mut self) {
        // A source is described only when two of them collide.
        enum Src {
            Process(ScopeId),
            Port(ObjId),
        }
        // Only unresolved leaves can collide: a flat list, in source order within a leaf.
        let mut src: Vec<(LeafId, Range, Src)> = Vec::new();
        let unresolved = |l: LeafId| self.m.nets.leaf(l).resolution.is_none();
        for d in &self.m.nets.drivers {
            if unresolved(d.leaf) {
                let p = &self.procs[d.proc.index()];
                src.push((d.leaf, d.range, Src::Process(p.scope)));
            }
        }
        let nets = &self.m.nets;
        let edges = nets
            .ports
            .iter()
            .map(|e| (e.mode, e.formal.0, vec![e.actual]));
        let convs = nets
            .conv_ports
            .iter()
            .map(|e| (e.mode, e.formal[0].0, e.actual.clone()));
        for (mode, formal, actual) in edges.chain(convs) {
            if mode != PortMode::In {
                let f = nets.leaf(formal).obj;
                for (l, r) in actual {
                    if unresolved(l) {
                        src.push((l, r, Src::Port(f)));
                    }
                }
            }
        }
        src.sort_by_key(|e| e.0);
        for group in src.chunk_by(|a, b| a.0 == b.0) {
            let leaf = self.m.nets.leaf(group[0].0);
            let v: Vec<(&Range, &Src)> = group.iter().map(|e| (&e.1, &e.2)).collect();
            for i in 0..v.len() {
                for j in i + 1..v.len() {
                    if v[i].0.overlaps(*v[j].0) {
                        let o = self.m.nets.obj(leaf.obj);
                        let span = self.db.decls[o.decl].span;
                        let describe = |s: &Src| match s {
                            Src::Process(scope) => format!(
                                "source: process {}",
                                self.m.hier.path_name(*scope, Case::Original)
                            ),
                            Src::Port(obj) => {
                                let f = self.m.nets.obj(*obj);
                                format!(
                                    "source: port {}{}",
                                    self.m.hier.path_name(f.scope, Case::Original),
                                    f.name.text
                                )
                            }
                        };
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
                            .with_note(describe(v[i].1))
                            .with_note(describe(v[j].1))
                            .with_help("drive it from one process, or use a resolved subtype such as `std_logic`"),
                        );
                        return;
                    }
                }
            }
        }
    }
}

/// The outermost composites resolved as a whole in `s`: (field path, instances, resolution
/// function, subtype), `outer` instances of `s` above.
fn whole_nodes(
    s: &Shape,
    path: &mut Vec<u32>,
    outer: u64,
    out: &mut Vec<(Vec<u32>, u64, DeclId, Shape)>,
) {
    match s {
        Shape::Array { res: Some(f), .. } | Shape::Record { res: Some(f), .. } => {
            out.push((path.clone(), outer, *f, s.clone()));
        }
        Shape::Scalar { .. } => {}
        Shape::Array { elem, .. } => whole_nodes(elem, path, outer * s.len(), out),
        Shape::Record { fields, .. } => {
            for (i, (_, f)) in fields.iter().enumerate() {
                path.push(i as u32);
                whole_nodes(f, path, outer, out);
                path.pop();
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

    /// Write the static value `v` of shape `shape` into the initial values of the leaf parts
    /// `parts` (a formal's part associated with a static expression).
    fn part_init(
        &mut self,
        db: &Db,
        parts: &[(LeafId, Range)],
        shape: &Shape,
        v: &Val,
        at: Span,
    ) -> R<()> {
        let vals = leaf_values(db, shape, Some(v)).map_err(|m| {
            Box::new(
                Diagnostic::error(codes::EVAL_FAILED, format!("actual does not fit: {m}"))
                    .with_primary(at, "here"),
            )
        })?;
        for (&(l, r), part) in parts.iter().zip(vals) {
            let (off, n) = (r.offset as usize, r.len as usize);
            match (&mut self.nets.leaves[l.index()].init, part) {
                (yavs_kernel::Value::Bytes(d), yavs_kernel::Value::Bytes(p)) if n > 0 => {
                    let es = p.len() / n;
                    d[off * es..(off + n) * es].copy_from_slice(&p);
                }
                (yavs_kernel::Value::Logic(d), yavs_kernel::Value::Logic(p)) => {
                    let old = d.clone();
                    *d = yavs_kernel::LogicVec::from_fn(old.len(), |i| {
                        if (off..off + n).contains(&i) {
                            p.get(i - off)
                        } else {
                            old.get(i)
                        }
                    });
                }
                (yavs_kernel::Value::Bytes(_), yavs_kernel::Value::Bytes(_)) => {}
                _ => return Err(Box::new(ice(at, "initial value of another element kind"))),
            }
        }
        Ok(())
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
        // Composites resolved as a whole, the signal or its outermost resolved subelements:
        // their leaves are resolved together, resolution below them ignored (#193; LRM
        // 14.7.3.1: a subelement of a resolved signal is not a basic signal).
        let mut wholes = Vec::new();
        whole_nodes(&shape, &mut Vec::new(), 1, &mut wholes);
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
            let whole = wholes.iter().find(|w| path.starts_with(&w.0));
            let resolution = match res {
                _ if whole.is_some() => whole.map(|w| LeafRes::Whole(w.2)),
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
                disconnect: yavs_kernel::Time::ZERO,
                implicit: None,
                implicit_more: Vec::new(),
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
            part: None,
        });
        for (path, outer, func, sub) in wholes {
            // The parts of instance i of the resolved subelement, as `NetGraph::select` maps them.
            let instances = (0..outer)
                .map(|i| {
                    (first.0..end.0)
                        .map(LeafId)
                        .filter(|&l| self.nets.leaf(l).path.starts_with(&path))
                        .filter_map(|l| {
                            let ty = self.nets.leaf(l).ty;
                            let k = ty.len / outer as u32;
                            let a = match ty.elem {
                                Elem::Logic => ty.len - (i as u32 + 1) * k,
                                Elem::Bytes(_) => i as u32 * k,
                            };
                            (k > 0).then(|| (l, Range::new(a, k)))
                        })
                        .collect::<Vec<_>>()
                })
                .filter(|parts| !parts.is_empty())
                .collect();
            self.nets.wholes.push(Whole {
                func,
                shape: sub,
                instances,
            });
        }
        self.hier.scopes[s.index()].objs.push(id);
        self.sigs.insert((s, decl), id);
        Ok(id)
    }

    /// The signal object and selectors a static signal name denotes, or `None` if `e`
    /// is not a signal name.
    fn name_ref(&mut self, db: &Db, e: ExprId, scope: ScopeId) -> R<Option<(ObjId, Vec<Sel>)>> {
        let ex = &db.exprs[e];
        let (prefix, sel) = match &ex.kind {
            // An object alias denotes the aliased name (LRM 6.6.2).
            ExprKind::Obj(d) => match &db.decls[*d].kind {
                DeclKind::Alias(Alias::Object { target, .. }) => {
                    return self.name_ref(db, *target, scope);
                }
                _ => {
                    let o = self.signal_or_package(db, scope, *d, ex.span)?;
                    return Ok(o.map(|o| (o, Vec::new())));
                }
            },
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
        let Some((o, mut v)) = self.name_ref(db, prefix, scope)? else {
            return Ok(None);
        };
        let sel = match &db.exprs[prefix].kind {
            ExprKind::Obj(a) => match &db.decls[*a].kind {
                DeclKind::Alias(Alias::Object {
                    subtype: Some(st), ..
                }) => self.rebase(db, o, &v, *st, sel, scope, ex.span)?,
                _ => sel,
            },
            _ => sel,
        };
        v.push(sel);
        Ok(Some((o, v)))
    }

    /// Selector `sel`, written in the index space of alias subtype `st`, as a selector of
    /// the aliased part `sels` of `o` (LRM 6.6.2: element positions correspond).
    #[allow(clippy::too_many_arguments)]
    fn rebase(
        &mut self,
        db: &Db,
        o: ObjId,
        sels: &[Sel],
        st: yavs_sem::SubtypeId,
        sel: Sel,
        scope: ScopeId,
        span: Span,
    ) -> R<Sel> {
        let Some(Shape::Array { dims: from, .. }) = self.shape(db, st, Some(scope), span)? else {
            return Ok(sel); // an unconstrained alias subtype keeps the name's bounds
        };
        let Shape::Array { dims: to, .. } = self.select(o, sels, span)?.1 else {
            return Err(Box::new(ice(
                span,
                "alias of an array subtype names a non-array",
            )));
        };
        let bad = |m: String| {
            Box::new(
                Diagnostic::error(codes::BAD_NAME, format!("invalid static name: {m}"))
                    .with_primary(span, "this name"),
            )
        };
        let map = |k: usize, v: i64| -> R<i64> {
            let (f, t) = (from[k], to[k]);
            let p = f.pos(v).ok_or_else(|| {
                bad(format!(
                    "index {v} is outside the alias range {} to {}",
                    f.left, f.right
                ))
            })?;
            Ok(crate::hier::index_at(t, p))
        };
        Ok(match sel {
            Sel::Index(xs) if xs.len() == to.len() && from.len() == to.len() => Sel::Index(
                xs.iter()
                    .enumerate()
                    .map(|(k, &v)| map(k, v))
                    .collect::<R<_>>()?,
            ),
            Sel::Slice(b) if to.len() == 1 && from.len() == 1 => {
                let t = to[0];
                if b.is_empty() {
                    let right = match t.dir {
                        yavs_sem::types::Direction::To => t.left - 1,
                        yavs_sem::types::Direction::Downto => t.left + 1,
                    };
                    Sel::Slice(crate::hier::Bounds {
                        left: t.left,
                        dir: t.dir,
                        right,
                    })
                } else {
                    Sel::Slice(crate::hier::Bounds {
                        left: map(0, b.left)?,
                        dir: t.dir,
                        right: map(0, b.right)?,
                    })
                }
            }
            _ => {
                return Err(Box::new(ice(
                    span,
                    "alias selector does not match its subtype",
                )));
            }
        })
    }

    /// Bind external signal name `x` (LRM 8.7) against the hierarchy elaborated so far
    /// (sim2 #548: a target that is not elaborated yet is an error, never a guess).
    pub(crate) fn external(
        &mut self,
        db: &Db,
        x: yavs_sem::ExternalId,
        scope: ScopeId,
    ) -> R<ObjId> {
        match self.external_target(db, x, scope)? {
            ExtTarget::Signal(o) => Ok(o),
            ExtTarget::Value(..) => Err(Box::new(ice(
                db.externals[x].span,
                "external signal name bound to a constant or variable",
            ))),
        }
    }

    /// Bind alias `d` of external name `x` declared or used in scope `s`: the alias of a
    /// signal is the signal object of its region, of a constant the constant's value, of a
    /// shared variable that variable (`ext_vars`). A process's alias belongs to its region.
    pub(crate) fn bind(
        &mut self,
        db: &Db,
        s: ScopeId,
        d: DeclId,
        x: yavs_sem::ExternalId,
    ) -> R<()> {
        let mut r = s;
        while let ScopeKind::Process { .. } = self.hier.scope(r).kind {
            r = self.hier.scope(r).parent.expect("a process has a region");
        }
        match self.external_target(db, x, s)? {
            ExtTarget::Signal(o) => {
                self.sigs.insert((r, d), o);
            }
            ExtTarget::Value(t, td) if db.externals[x].class == ExternalClass::Constant => {
                let mark = self.session.mark();
                match (self.value_of(db, td, t, db.externals[x].span), t) {
                    (Ok(v), _) => self.set(r, d, v),
                    // A constant only its run-time thunk computes (`'path_name`, say): the
                    // alias takes the target's value when the simulation is built (#255).
                    (Err(e), Some(t)) if e.code == codes::NOT_EVALUATED => {
                        self.session
                            .undo(db, mark)
                            .map_err(|m| Box::new(ice(db.externals[x].span, &m)))?;
                        self.ext_consts.insert((r, d), (t, td));
                    }
                    (Err(e), _) => return Err(e),
                }
            }
            ExtTarget::Value(Some(t), td) => {
                self.ext_vars.insert((r, d), (t, td));
            }
            ExtTarget::Value(None, _) => {
                return Err(Box::new(ice(
                    db.externals[x].span,
                    "package shared variable",
                )));
            }
        }
        Ok(())
    }

    /// The value of external constant name `x` (LRM 8.7), bound now.
    pub(crate) fn external_value(
        &mut self,
        db: &Db,
        x: yavs_sem::ExternalId,
        scope: Option<ScopeId>,
    ) -> R<Val> {
        let span = db.externals[x].span;
        if db.externals[x].class != ExternalClass::Constant {
            return Err(Box::new(
                Diagnostic::error(
                    codes::NOT_EVALUATED,
                    "an external signal or variable has no value during elaboration",
                )
                .with_primary(span, "not a constant"),
            ));
        }
        let target = match scope {
            Some(s) => self.external_target(db, x, s)?,
            None if db.externals[x].path_kind == yavs_syntax::ast::PathKind::Package => {
                self.external_target(db, x, ScopeId(0))?
            }
            None => {
                return Err(unsupported(
                    span,
                    "external names outside the design hierarchy (in a package)",
                    "M3",
                ));
            }
        };
        match target {
            ExtTarget::Value(t, d) if db.externals[x].class == ExternalClass::Constant => {
                self.value_of(db, d, t, span)
            }
            _ => Err(Box::new(ice(
                span,
                "external constant name bound to a non-constant",
            ))),
        }
    }

    /// Bind external name `x` (LRM 8.7) against the hierarchy elaborated so far: a signal
    /// object, or the region and declaration of a constant or shared variable.
    pub(crate) fn external_target(
        &mut self,
        db: &Db,
        x: yavs_sem::ExternalId,
        scope: ScopeId,
    ) -> R<ExtTarget> {
        let t = self.find_external(db, x, scope)?;
        self.check_external(db, x, scope, &t)?;
        Ok(t)
    }

    /// The type of the object an external name denotes is the type of its subtype indication,
    /// and an array's constrained dimensions have the object's lengths (LRM 8.7): a mismatch
    /// is an error when the name is bound.
    fn check_external(
        &mut self,
        db: &Db,
        x: yavs_sem::ExternalId,
        scope: ScopeId,
        t: &ExtTarget,
    ) -> R<()> {
        let ext = &db.externals[x];
        let want = db.base(ext.subtype);
        if matches!(db.types[want].kind, yavs_sem::types::TypeKind::Error) {
            return Ok(());
        }
        let (name, class, got, shape) = match *t {
            ExtTarget::Signal(o) => {
                let obj = self.nets.obj(o);
                (obj.name.text.to_string(), "signal", obj.shape.ty(), Some(o))
            }
            ExtTarget::Value(_, d) => {
                let DeclKind::Object(o) = &db.decls[d].kind else {
                    return Ok(());
                };
                let class = if ext.class == ExternalClass::Constant {
                    "constant"
                } else {
                    "shared variable"
                };
                (
                    db.decls[d].name.to_string(),
                    class,
                    db.base(o.subtype),
                    None,
                )
            }
        };
        let fail = |why: String| {
            Box::new(
                Diagnostic::error(
                    codes::EXTERNAL_MISMATCH,
                    format!("external name does not match {class} `{name}`: {why}"),
                )
                .with_primary(ext.span, "external name")
                .with_note("the object has the type and the array lengths of the name's subtype indication (LRM 8.7)"),
            )
        };
        if want != got {
            return Err(fail(format!(
                "its type is `{}`, the name says `{}`",
                db.type_name(got),
                db.type_name(want)
            )));
        }
        let Some(o) = shape else { return Ok(()) };
        let Some(sub) = self.shape(db, ext.subtype, Some(scope), ext.span)? else {
            return Ok(()); // unconstrained: the object's bounds
        };
        match length_mismatch(&sub, &self.nets.obj(o).shape) {
            Some((name_n, obj_n)) => Err(fail(format!(
                "the object has {obj_n} elements, the subtype indication {name_n}"
            ))),
            None => Ok(()),
        }
    }

    fn find_external(&mut self, db: &Db, x: yavs_sem::ExternalId, scope: ScopeId) -> R<ExtTarget> {
        use yavs_syntax::ast::PathKind;
        let ext = &db.externals[x];
        let span = ext.span;
        let path: Vec<String> = ext.path.iter().map(|(i, _)| i.text.to_string()).collect();
        let shown = path.join(".");
        let class = ext.class;
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
        let Some(((last, _), steps)) = ext.path.split_last() else {
            return Err(fail("empty path".into()));
        };
        let what = match class {
            ExternalClass::Signal => "signal",
            ExternalClass::Constant => "constant",
            ExternalClass::Variable => "shared variable",
        };
        // A constant or shared variable `d` of the right class (LRM 8.7).
        let value_decl = |d: DeclId| match &db.decls[d].kind {
            DeclKind::Object(o) => matches!(
                (class, o.class),
                (ExternalClass::Constant, ObjClass::Constant)
                    | (ExternalClass::Variable, ObjClass::SharedVariable)
            ),
            _ => false,
        };
        if ext.path_kind == PathKind::Package {
            // `@lib.pkg.obj`: the object declared in the package.
            let d = db.package_path_target(x).map_err(fail)?;
            if class != ExternalClass::Signal {
                return Ok(ExtTarget::Value(None, d));
            }
            return self
                .signal_or_package(db, scope, d, span)?
                .map(ExtTarget::Signal)
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
        let missing = |m: &Self| {
            fail(format!(
                "{} has no {what} `{}` (yet)",
                m.hier.path_name(cur, Case::Original),
                last.text
            ))
        };
        if class != ExternalClass::Signal {
            let d = self.regions.get(&cur).into_iter().flatten().find_map(|&r| {
                db.scopes[r]
                    .names
                    .get(&last.key)?
                    .iter()
                    .copied()
                    .find(|&d| value_decl(d))
            });
            return d
                .map(|d| ExtTarget::Value(Some(cur), d))
                .ok_or_else(|| missing(self));
        }
        self.hier
            .scope(cur)
            .objs
            .iter()
            .copied()
            .find(|&o| self.nets.obj(o).name.key == last.key)
            .map(ExtTarget::Signal)
            .ok_or_else(|| missing(self))
    }

    /// The hierarchy scope of package `unit`, created on first use.
    pub(crate) fn pkg_scope(&mut self, db: &Db, unit: UnitId) -> ScopeId {
        if let Some(&p) = self.pkg_scopes.get(&unit) {
            return p;
        }
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
        // The package unit, through the package instances declared in it (#101).
        let mut region = decl.scope;
        let unit = loop {
            let Some(ow) = db.scopes[region].owner else {
                return Ok(None);
            };
            match db.decls[ow].kind {
                DeclKind::Unit(u)
                    if matches!(db.units[u].kind, UnitKind::Package | UnitKind::PackageInst) =>
                {
                    break u;
                }
                DeclKind::PackageInst(_) | DeclKind::Package { .. } => region = db.decls[ow].scope,
                _ => return Ok(None),
            }
        };
        let ps = self.pkg_scope(db, unit);
        if let Some(&obj) = self.sigs.get(&(ps, d)) {
            return Ok(Some(obj));
        }
        if o.signal_kind.is_some() {
            return Err(unsupported(decl.span, "guarded signals in packages", "M3"));
        }
        let init = match o.default {
            Some(e) => Some(self.eval(db, e, None)?),
            None => None,
        };
        let Some(shape) = self.shape_init(db, o.subtype, init.as_ref(), None, decl.span)? else {
            return Err(unconstrained_signal(at));
        };
        let obj = self.add_obj(db, ps, d, ObjKind::Signal, shape, init.as_ref(), decl.span)?;
        Ok(Some(obj))
    }

    fn lsp_ref(&mut self, db: &Db, l: &Lsp, scope: ScopeId) -> R<(ObjId, Vec<Sel>)> {
        let d = match l.root {
            ObjRoot::Decl(d) => d,
            ObjRoot::External(x) => {
                let o = self.external(db, x, scope)?;
                let sels = self.steps(db, Some(o), &l.path, scope, l.span)?;
                return Ok((o, sels));
            }
            ObjRoot::ImplicitSignal(_) => {
                return Err(unsupported(l.span, "implicit signals as targets", "M3"));
            }
        };
        let Some(o) = self.signal_or_package(db, scope, d, l.span)? else {
            return Err(Box::new(ice(l.span, "driven signal without object")));
        };
        let sels = self.steps(db, Some(o), &l.path, scope, l.span)?;
        Ok((o, sels))
    }

    /// The shape of an unconstrained port (or subelement of one) of subtype `st` associated
    /// individually; `parts` are the paths of the formal parts below it, with their actuals
    /// (LRM 6.5.7.1): a subelement associated as a whole takes its actual's bounds, a record
    /// shapes each element from the parts naming it, and an array runs per dimension from the
    /// lowest to the highest index named, in the direction of the index subtype (as the
    /// reference simulators do).
    #[allow(clippy::too_many_arguments)]
    fn parts_shape(
        &mut self,
        db: &Db,
        s: ScopeId,
        st: yavs_sem::SubtypeId,
        parts: &[(&[LspStep], ExprId)],
        sc: ScopeId,
        decl: &yavs_sem::decl::Decl,
    ) -> R<Shape> {
        let span = decl.span;
        if let Some(sh) = self.shape(db, st, Some(s), span)? {
            return Ok(sh);
        }
        if let [([], e)] = parts {
            return self.actual_shape(db, s, st, *e, sc, decl);
        }
        let ty = db.base(st);
        let res = match db.subtypes[st].resolution {
            Some(yavs_sem::types::Resolution::Function(f)) => Some(f),
            _ => None,
        };
        let (indexes, elem) = match &db.types[ty].kind {
            yavs_sem::types::TypeKind::Array { indexes, elem } => (indexes, *elem),
            yavs_sem::types::TypeKind::Record { elems }
                if matches!(
                    db.subtypes[st].constraint,
                    yavs_sem::types::Constraint::None
                ) =>
            {
                let mut fields = Vec::with_capacity(elems.len());
                for (i, f) in elems.iter().enumerate() {
                    let sub: Vec<_> = parts
                        .iter()
                        .filter_map(|&(steps, e)| match steps.first() {
                            Some(LspStep::Field(k)) if *k as usize == i => Some((&steps[1..], e)),
                            _ => None,
                        })
                        .collect();
                    if sub.is_empty() && self.shape(db, f.subtype, Some(s), span)?.is_none() {
                        // ponytail: sem does not yet reject a subelement left out of an
                        // individual association (LRM 6.5.7.1); only unconstrained ones fail here.
                        return Err(Box::new(
                            Diagnostic::error(
                                codes::PORT_UNCONSTRAINED,
                                format!(
                                    "element `{}` of port `{}` is unconstrained and not associated",
                                    f.name.text, decl.name
                                ),
                            )
                            .with_primary(span, "port declared here")
                            .with_note("LRM 6.5.7.1: an individual association names every subelement of the formal"),
                        ));
                    }
                    fields.push((f.name, self.parts_shape(db, s, f.subtype, &sub, sc, decl)?));
                }
                return Ok(Shape::Record { ty, fields, res });
            }
            _ => {
                return Err(unsupported(
                    span,
                    "individual association of this unconstrained port",
                    "M3",
                ));
            }
        };
        let mut lh: Vec<Option<(i64, i64)>> = vec![None; indexes.len()];
        for &(steps, e) in parts {
            let espan = db.exprs[e].span;
            let Some(first) = self
                .steps(db, None, &steps[..1.min(steps.len())], sc, espan)?
                .pop()
            else {
                return Err(Box::new(ice(espan, "empty sub-element path")));
            };
            let ranges: Vec<(i64, i64)> = match first {
                Sel::Index(xs) => xs.iter().map(|&x| (x, x)).collect(),
                Sel::Slice(b) if !b.is_empty() => vec![b.low_high()],
                Sel::Slice(_) => continue,
                Sel::Field(_) => return Err(Box::new(ice(espan, "field of an array port"))),
            };
            for (d, (lo, hi)) in ranges.into_iter().enumerate() {
                let cur = &mut lh[d];
                *cur = Some(cur.map_or((lo, hi), |(a, b)| (a.min(lo), b.max(hi))));
            }
        }
        let mut dims = Vec::with_capacity(indexes.len());
        for (d, &ix) in indexes.iter().enumerate() {
            let dir = self.scalar_bounds(db, ix, Some(s), span)?.dir;
            let Some((lo, hi)) = lh[d] else {
                return Err(unsupported(
                    span,
                    "an individually associated port with only null slices",
                    "M3",
                ));
            };
            dims.push(match dir {
                yavs_sem::types::Direction::To => crate::hier::Bounds {
                    left: lo,
                    dir,
                    right: hi,
                },
                yavs_sem::types::Direction::Downto => crate::hier::Bounds {
                    left: hi,
                    dir,
                    right: lo,
                },
            });
        }
        let Some(es) = self.shape(db, elem, Some(s), span)? else {
            return Err(unsupported(
                span,
                "individual association of a port with unconstrained elements",
                "M3",
            ));
        };
        Ok(Shape::Array {
            ty,
            dims,
            elem: Box::new(es),
            res,
        })
    }

    /// The shape of actual `e` (in region `sc`) of an unconstrained subelement of subtype `st`
    /// of port `decl` of instance `s`: a signal name's, else the expression's subtype's, else its static value's.
    fn actual_shape(
        &mut self,
        db: &Db,
        s: ScopeId,
        st: yavs_sem::SubtypeId,
        e: ExprId,
        sc: ScopeId,
        decl: &yavs_sem::decl::Decl,
    ) -> R<Shape> {
        let espan = db.exprs[e].span;
        if let Some((ao, asels)) = self.name_ref(db, e, sc)? {
            return Ok(self.select(ao, &asels, espan)?.1);
        }
        if let Some(sh) = self.shape(db, db.exprs[e].subtype, Some(sc), espan)? {
            return Ok(sh);
        }
        let v = match self.eval(db, e, Some(sc)) {
            Err(d) if d.code == codes::NOT_EVALUATED => {
                return Err(not_static_subtype(espan, decl.span, decl));
            }
            r => r?,
        };
        self.shape_init(db, st, Some(&v), Some(s), espan)?
            .ok_or_else(|| not_static_subtype(espan, decl.span, decl))
    }

    /// The selectors of path `path` of object `o` (`None` for a formal's path, which never
    /// goes through an alias).
    fn steps(
        &mut self,
        db: &Db,
        o: Option<ObjId>,
        path: &[LspStep],
        scope: ScopeId,
        span: Span,
    ) -> R<Vec<Sel>> {
        let mut v = Vec::with_capacity(path.len());
        let mut rebase = None;
        for st in path {
            let sel = match st {
                LspStep::Rebase(s) => {
                    rebase = Some(*s);
                    continue;
                }
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
            };
            let sel = match (rebase.take(), o) {
                (Some(s), Some(o)) => self.rebase(db, o, &v, s, sel, scope, span)?,
                (Some(_), None) => return Err(Box::new(ice(span, "alias step in a formal"))),
                (None, _) => sel,
            };
            v.push(sel);
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

    /// The port mode of each leaf of port object `fo` with mode `m`: one for all, or per
    /// record element from a mode view.
    fn leaf_modes(&self, db: &Db, fo: ObjId, m: Mode, span: Span) -> R<HashMap<LeafId, PortMode>> {
        let obj = self.nets.obj(fo);
        let (a, _) = obj.leaves;
        let tree = match m {
            Mode::View { .. } => Some(db.port_mode_tree(m)),
            _ => None,
        };
        let mut out = HashMap::new();
        for (k, (path, _, _)) in obj.shape.leaves().into_iter().enumerate() {
            let mode = match &tree {
                None => port_mode(m),
                Some(t) => {
                    let mut t = t;
                    let mut p = path.as_slice();
                    loop {
                        match t {
                            yavs_sem::view::ModeTree::Leaf(m) => break port_mode(*m),
                            yavs_sem::view::ModeTree::Array(e) => t = e,
                            yavs_sem::view::ModeTree::Record { elems, .. } => {
                                let Some((&f, rest)) = p.split_first() else {
                                    return Err(Box::new(ice(
                                        span,
                                        "mode view deeper than the port",
                                    )));
                                };
                                let Some(e) = elems.get(f as usize) else {
                                    return Err(Box::new(ice(
                                        span,
                                        "mode view does not fit the port",
                                    )));
                                };
                                t = e;
                                p = rest;
                            }
                        }
                    }
                }
            };
            out.insert(LeafId(a.0 + k as u32), mode);
        }
        Ok(out)
    }

    fn edges(
        &mut self,
        fl: &[(LeafId, Range)],
        al: &[(LeafId, Range)],
        modes: &HashMap<LeafId, PortMode>,
        span: Span,
    ) -> R<()> {
        self.same_parts(fl, al, span)?;
        for (f, a) in fl.iter().zip(al) {
            if f.1.len == 0 {
                continue;
            }
            let Some(&mode) = modes.get(&f.0) else {
                return Err(Box::new(ice(span, "formal leaf without a mode")));
            };
            // An `in` formal starts with its actual's value (LRM 14.7.3.3), which the
            // initial value of a later signal may read.
            if mode == PortMode::In {
                let al = self.nets.leaf(a.0);
                let src = crate::collapse::slice_init(&al.init, al.ty.len, a.1);
                crate::collapse::put_init(
                    &mut self.nets.leaves[f.0.index()].init,
                    f.1.offset,
                    &src,
                    f.1.len,
                );
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

    /// Formal and actual leaf parts pair up one to one (`PORT_MISMATCH` otherwise).
    fn same_parts(&self, fl: &[(LeafId, Range)], al: &[(LeafId, Range)], span: Span) -> R<()> {
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
        Ok(())
    }
}

/// The first pair of array dimension lengths (the name's, the object's) that differ.
fn length_mismatch(name: &Shape, obj: &Shape) -> Option<(u64, u64)> {
    match (name, obj) {
        (
            Shape::Array {
                dims: a, elem: ea, ..
            },
            Shape::Array {
                dims: b, elem: eb, ..
            },
        ) => a
            .iter()
            .zip(b)
            .find(|(x, y)| x.len() != y.len())
            .map(|(x, y)| (x.len(), y.len()))
            .or_else(|| length_mismatch(ea, eb)),
        (Shape::Record { fields: a, .. }, Shape::Record { fields: b, .. }) => a
            .iter()
            .zip(b)
            .find_map(|((_, x), (_, y))| length_mismatch(x, y)),
        _ => None,
    }
}

/// What an external name denotes (LRM 8.7).
pub(crate) enum ExtTarget {
    Signal(ObjId),
    /// A constant or shared variable: the region declaring it (`None`: a package) and its
    /// declaration.
    Value(Option<ScopeId>, DeclId),
}

/// A signal whose subtype is not fully constrained and whose initial value does not give
/// the missing bounds: sem rejects it, so this is a bug.
fn unconstrained_signal(span: Span) -> Box<Diagnostic> {
    Box::new(ice(span, "signal of an unconstrained subtype"))
}

/// The kernel mode of a simple port mode. A linkage port takes the actual's value like an
/// `in` port: VHDL code cannot read or update it (LRM 6.5.2), so it never drives the actual.
/// The subtype of the scalar part `steps` of an object of subtype `st`; `None` for a
/// composite part.
fn scalar_part(
    db: &Db,
    mut st: yavs_sem::SubtypeId,
    steps: &[LspStep],
) -> Option<yavs_sem::SubtypeId> {
    use yavs_sem::types::TypeKind;
    for step in steps {
        st = match (step, &db.types[db.base(st)].kind) {
            (LspStep::Field(i), TypeKind::Record { elems }) => elems.get(*i as usize)?.subtype,
            (LspStep::Index(_), TypeKind::Array { elem, .. }) => *elem,
            _ => return None,
        };
    }
    db.types[db.base(st)].kind.is_scalar().then_some(st)
}

fn port_mode(m: Mode) -> PortMode {
    match m {
        Mode::In | Mode::Linkage | Mode::View { .. } => PortMode::In,
        Mode::Out => PortMode::Out,
        Mode::Inout => PortMode::Inout,
        Mode::Buffer => PortMode::Buffer,
    }
}

/// The type and bounds of the elements of a scalar or one-dimensional array of std_ulogic, BIT or
/// BOOLEAN.
fn logic_elem(shape: &Shape) -> Option<(yavs_sem::TypeId, crate::hier::Bounds)> {
    let elem = match shape {
        Shape::Scalar { .. } => shape,
        Shape::Array { dims, elem, .. } if dims.len() == 1 => &**elem,
        _ => return None,
    };
    match elem {
        Shape::Scalar {
            ty,
            range: Some(range),
            elem: Elem::Logic,
            ..
        } => Some((*ty, *range)),
        _ => None,
    }
}

/// `same_values` of two arrays that may be of different types (the operand and the result of a
/// conversion between arrays): the same lengths per dimension and elements of the same values.
fn same_array_values(a: &Shape, b: &Shape) -> bool {
    match (a, b) {
        (
            Shape::Array {
                dims: da, elem: ea, ..
            },
            Shape::Array {
                dims: db, elem: eb, ..
            },
        ) => {
            da.len() == db.len()
                && da.iter().zip(db).all(|(x, y)| x.len() == y.len())
                && same_values(ea, eb)
        }
        _ => false,
    }
}

/// Whether every value of shape `a` is a value of shape `b` with the same elements: the same
/// types, array lengths per dimension, and scalar subtypes or bounds.
fn same_values(a: &Shape, b: &Shape) -> bool {
    match (a, b) {
        (
            Shape::Scalar {
                ty: ta,
                sub: sa,
                range: ra,
                ..
            },
            Shape::Scalar {
                ty: tb,
                sub: sb,
                range: rb,
                ..
            },
        ) => ta == tb && (sa == sb || ra.is_some() && ra == rb),
        (
            Shape::Array {
                ty: ta,
                dims: da,
                elem: ea,
                ..
            },
            Shape::Array {
                ty: tb,
                dims: db,
                elem: eb,
                ..
            },
        ) => {
            ta == tb
                && da.len() == db.len()
                && da.iter().zip(db).all(|(x, y)| x.len() == y.len())
                && same_values(ea, eb)
        }
        (
            Shape::Record {
                ty: ta, fields: fa, ..
            },
            Shape::Record {
                ty: tb, fields: fb, ..
            },
        ) => {
            ta == tb
                && fa.len() == fb.len()
                && fa.iter().zip(fb).all(|((_, x), (_, y))| same_values(x, y))
        }
        _ => false,
    }
}
