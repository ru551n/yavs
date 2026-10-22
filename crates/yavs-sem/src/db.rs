//! The session database: arenas, well-known entities, diagnostics (sem.md 1.1).
//!
//! One `Db` per session. Ids are deterministic for a given input because
//! analysis order is deterministic; they are not stable across inputs.

use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use std::path::PathBuf;

use yavs_diag::{Diagnostic, Severity, SourceMap, Span};
use yavs_syntax::Symbol;
use yavs_syntax::ast::Ident;

use crate::attr::AttrValue;
use crate::decl::{Decl, DeclKind, Designator};
use crate::diag;
use crate::generic::Instance;
use crate::hir::{Expr, ExprKind, External, Stmt};
use crate::ids::*;
use crate::loader::{FileCache, Library, Unit};
use crate::scope::{Scope, ScopeKind};
use crate::stat::{ConstValue, Staticness};
use crate::types::{Constraint, SubtypeDef, TypeDef, TypeKind};
use crate::view::ViewDef;
use crate::{LangRev, Options};

/// Entities of `std.standard` that analysis refers to directly. Filled while
/// `std.standard` is analyzed; `None` before that.
#[derive(Default, Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct WellKnown {
    pub universal_integer: Option<TypeId>,
    pub universal_real: Option<TypeId>,
    pub boolean: Option<TypeId>,
    pub bit: Option<TypeId>,
    pub character: Option<TypeId>,
    pub severity_level: Option<TypeId>,
    pub integer: Option<TypeId>,
    pub real: Option<TypeId>,
    pub time: Option<TypeId>,
    pub string: Option<TypeId>,
    pub natural: Option<SubtypeId>,
    pub positive: Option<SubtypeId>,
    pub delay_length: Option<SubtypeId>,
    pub file_open_kind: Option<TypeId>,
    pub file_open_status: Option<TypeId>,
    /// 2019 `RANGE_DIRECTION` (LRM 16.3; `lib` renames the package's `DIRECTION`).
    pub range_direction: Option<TypeId>,
    /// The `std.standard` package scope.
    pub standard: Option<ScopeId>,
}

/// Poison entities (sem.md 8.2): compatible with everything, silent in checks.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct Poison {
    pub decl: DeclId,
    pub ty: TypeId,
    pub subtype: SubtypeId,
    pub expr: ExprId,
}

fn placeholder_options() -> Options {
    Options::new(LangRev::Vhdl2008, "")
}

/// Marker that an error was already reported; callers propagate it silently.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reported;

/// The unit currently being analyzed (on-demand loading nests these).
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
pub(crate) struct UnitCx {
    pub unit: UnitId,
    pub lib: LibId,
    /// The file's own revision `F` (sem.md 0.1).
    pub file_rev: LangRev,
}

/// What the elaborator wants to know about every analyzed expression and statement, found once
/// when the analysis is settled so that a `Db` restored from an image need not decode all of
/// them to answer ([`Db::facts`]).
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Facts {
    /// Sizes of the arenas the facts were found in; a `Db` that grew since recomputes them.
    exprs: usize,
    stmts: usize,
    /// Some expression reads `'active` or `'last_active` (used or not).
    pub reads_activity: bool,
    /// The force and release statements, in order.
    pub forces: Vec<StmtId>,
    /// The expressions that were package-path external names (`@lib.pkg.obj`) with their names.
    pub(crate) package_paths: Vec<(ExprId, ExternalId)>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Db {
    /// The design revision `D`: std and ieee are loaded for it.
    pub rev: LangRev,
    #[serde(skip, default = "placeholder_options")]
    pub(crate) opts: Options,
    pub sources: SourceMap,
    pub libs: Arena<LibId, Library>,
    pub units: Arena<UnitId, Unit>,
    pub decls: Arena<DeclId, Decl>,
    pub scopes: Arena<ScopeId, Scope>,
    pub types: Arena<TypeId, TypeDef>,
    pub subtypes: Arena<SubtypeId, SubtypeDef>,
    pub exprs: Arena<ExprId, Expr>,
    pub stmts: Arena<StmtId, Stmt>,
    pub consts: Arena<ConstId, ConstValue>,
    pub insts: Arena<InstId, Instance>,
    pub views: Arena<ViewId, ViewDef>,
    pub externals: Arena<ExternalId, External>,
    pub attr_values: Arena<AttrValueId, AttrValue>,
    /// User attribute values by (entity, attribute name key) (attr.rs).
    pub attr_index: HashMap<(DeclId, Symbol), AttrValueId>,
    /// 2019 method aliases `alias m is v.m;`: the alias to the object `v` (a private
    /// variable, a shared variable, a variable) whose method it forwards (5.6.2, 6.6.3).
    pub(crate) forwarding: HashMap<DeclId, DeclId>,
    /// Disconnection specifications (7.4): the time expression of each guarded signal that has
    /// one (a guarded signal without an entry disconnects after 0 fs).
    pub disconnect_times: HashMap<DeclId, ExprId>,
    /// 2019 named return values (`return r of t`, 4.2.1), by the function's parameter region:
    /// the hidden constant whose descriptor is the call's result subtype (M2 passes it after
    /// the parameters) and the subtype `r` it constrains.
    pub ret_names: HashMap<ScopeId, (DeclId, SubtypeId)>,
    /// Every non-zero TIME physical literal or unit name below one second that was analyzed,
    /// for the resolution limit check at elaboration (LRM 5.2.4.2, [`Db::fine_time_units`]).
    pub time_units: Vec<TimeUnitUse>,
    /// (region, attribute key, entity class) closed by `others`/`all` (7.2).
    pub(crate) attr_closed: HashSet<(ScopeId, Symbol, yavs_syntax::token::Kw)>,
    /// Specifications of labels of statements not analyzed yet (attr.rs, 7.2).
    #[serde(skip)]
    pub(crate) pending_label_attrs: Vec<crate::attr::PendingLabel>,
    /// Package instances in a process or subprogram whose generic's body is not declared yet
    /// (instance, region, span): `E1237` is checked when the body arrives.
    #[serde(skip)]
    pub(crate) pending_inst_checks: Vec<(crate::ids::InstId, ScopeId, yavs_diag::Span)>,
    /// Groups with a label constituent not declared yet.
    #[serde(skip)]
    pub(crate) pending_groups: Vec<crate::group::PendingGroup>,
    pub well_known: WellKnown,
    pub poison: Poison,
    pub(crate) root: ScopeId,
    /// `None`: the library is unknown and that was reported.
    pub(crate) lib_by_key: HashMap<Symbol, Option<LibId>>,
    /// Parsed files per library: a file analyzed into several libraries is parsed once for
    /// each, as each library's units are separate (#290).
    #[serde(skip)]
    pub(crate) files: HashMap<(LibId, PathBuf), FileCache>,
    #[serde(skip)]
    pub(crate) stack: Vec<UnitCx>,
    #[allow(dead_code)]
    /// Undeclared names already reported, per (unit, key) (8.2).
    pub(crate) undeclared: HashSet<(Option<UnitId>, Symbol)>,
    /// Spans already given `E1007` or `W1008` by rev.rs (once per construct: names are
    /// classified once per resolution pass).
    pub(crate) rev_reported: HashSet<Span>,
    /// Bottom-up candidate sets of the complete context being resolved, by AST node address.
    /// `lookup` results valid while `Db::cache_epoch` is unchanged: (scope, key) -> (revision, epoch, direct hit, result).
    #[serde(skip)]
    pub(crate) lookup_cache:
        HashMap<(ScopeId, Symbol), (LangRev, u64, bool, crate::scope::Visible)>,
    #[serde(skip)]
    pub(crate) memo: HashMap<usize, crate::expr::CandSet>,
    /// The same for name nodes.
    #[serde(skip)]
    pub(crate) name_memo: HashMap<usize, crate::expr::CandSet>,
    /// Per declaration (by index): what its first parameter's type says about the operands it
    /// can take, 0 when not worked out yet (`expr/cands.rs` `first_param_key`).
    #[serde(skip)]
    pub(crate) first_param: Vec<u32>,
    /// Emptied pairs of memos kept for the next complete context, so that its tables start
    /// with the capacity the last ones grew to.
    #[serde(skip)]
    pub(crate) memo_pool: Vec<(
        HashMap<usize, crate::expr::CandSet>,
        HashMap<usize, crate::expr::CandSet>,
    )>,
    /// Units that depend on a failed unit: undeclared names in them are cascades (8.2).
    pub(crate) tainted: HashSet<UnitId>,
    /// The subtype the next complete context's top expression is resolved for
    /// (an object's declared subtype), so an aggregate with `others` knows its
    /// bounds (9.3.3.3). Consumed by `resolve`.
    #[serde(skip)]
    pub(crate) context_subtype: Option<SubtypeId>,
    /// `(top AST node address, context subtype)` of the complete context being resolved.
    #[serde(skip)]
    pub(crate) top_context: Option<(usize, SubtypeId)>,
    /// Folds that overflowed in the complete context being resolved (Q5).
    /// `Some(why)` when the operation has no value (division by zero, ...).
    #[serde(skip)]
    pub(crate) overflow: Vec<(ExprId, Option<&'static str>)>,
    /// Bounds being resolved are required at analysis time (Q5: array type bounds).
    #[serde(skip)]
    pub(crate) required_ctx: bool,
    /// Generic types of the instance whose port map is being resolved, with their actuals.
    #[serde(skip)]
    pub(crate) inst_types: Vec<(TypeId, SubtypeId)>,
    /// The anonymous interface types (`type is ...`) of the objects of the interface list
    /// being analyzed; the owner takes them (entities append them to their generics).
    #[serde(skip)]
    pub(crate) hidden_types: Vec<DeclId>,
    /// The anonymous types of the parameters of the subprogram spec being analyzed.
    #[serde(skip)]
    pub(crate) param_anon: Vec<TypeId>,
    /// The anonymous parameter types of the spec a subprogram body being analyzed completes,
    /// taken in order by its parameters instead of new ones (`subprogram_body`).
    #[serde(skip)]
    pub(crate) anon_reuse: std::collections::VecDeque<SubtypeId>,
    /// External names of the current unit already built, by AST node address (built once:
    /// candidates need the subtype, the HIR is the same).
    #[serde(skip)]
    pub(crate) ext_memo: HashMap<usize, ExprId>,
    /// Implicit aliases of the external names used directly in a statement or an alias
    /// target, made by [`Db::alias_external_uses`] (#93).
    pub(crate) ext_uses: HashMap<crate::external::ExtOwner, Box<[DeclId]>>,
    /// The package object each resolved package-path external name denotes, made by
    /// [`Db::bind_package_paths`] (#255).
    pub(crate) pkg_targets: HashMap<ExternalId, DeclId>,
    /// The type mark as written of a parameter (by its object decl) and of a function's
    /// result (by the subprogram decl), for messages in the user's spelling (8.3).
    pub(crate) written_marks: HashMap<DeclId, Span>,
    /// A type mark may name an incomplete type: the designated subtype of an
    /// access type definition is being analyzed (alloc.rs, 5.4.2).
    #[serde(skip)]
    pub(crate) incomplete_ok: bool,
    /// Implicit operations waiting for a well-known type of `std.standard` (BOOLEAN, STRING, ...).
    #[serde(skip)]
    pub(crate) pending_implicit: Vec<crate::implicit::Pending>,
    /// How much of the arenas `validate` already checked.
    pub(crate) validated: crate::validate::Mark,
    /// Processes and subprograms whose statements are being analyzed, innermost last (stmt.rs).
    #[serde(skip)]
    pub(crate) stmt_owners: Vec<crate::stmt::Owner>,
    /// Configuration specifications (7.3.1) by the declarative region they are in (unit.rs).
    pub config_specs: HashMap<ScopeId, Vec<crate::hir::ComponentConfig>>,
    /// Configuration specifications whose instance labels wait for the statement part.
    #[serde(skip)]
    pub(crate) pending_specs: Vec<crate::unit::PendingSpec>,
    /// 2019 range record types (`R'RECORD`) by scalar base type, made on first use (attr.rs).
    pub(crate) range_records: HashMap<TypeId, SubtypeId>,
    /// Generic defaults, generic lists and instance remappings (generic.rs).
    pub(crate) generic: crate::generic::GenericState,
    /// Whole-design facts found by [`Db::settle`] (see [`Facts`]).
    pub(crate) facts: Option<Facts>,
    /// The packages outside std with run-time objects as [`Db::run_time_package_set`] found them
    /// when the image was made, with the sizes of the arenas they were found in (#426).
    pub(crate) run_time_pkgs: Option<((usize, usize, usize), Vec<UnitId>)>,
    /// The facts of the units replayed from the analysed-unit cache since the last settle, by
    /// load (their entries are not scanned; `units.rs`).
    #[serde(skip)]
    pub(crate) unit_facts: rustc_hash::FxHashMap<u32, Facts>,
    pub(crate) diags: Vec<Diagnostic>,
    pub(crate) errors: usize,
    /// Every load of a unit in this session, by the owner number of its arena entries
    /// (`units.rs`, the analysed-unit cache).
    pub(crate) loads: Vec<crate::units::LoadRec>,
    /// The load that registered each source file (`ids::SESSION` for none).
    pub(crate) file_owner: Vec<u32>,
    /// The units being analysed from source and recorded, innermost last.
    #[serde(skip)]
    pub(crate) frames: Vec<crate::units::Frame>,
    /// Questions about the libraries the innermost frame asked since its last step.
    #[serde(skip)]
    pub(crate) query_log: std::sync::Mutex<Vec<(crate::units::Query, crate::units::Answer)>>,
    /// How to undo the replay in progress (`units.rs`).
    #[serde(skip)]
    pub(crate) undo: Vec<crate::units::Undo>,
    /// The owner of the entries pushed now.
    #[serde(skip, default = "session_owner")]
    pub(crate) owner: u32,
    /// Depth of replays from the analysed-unit cache in progress.
    #[serde(skip)]
    pub(crate) replaying: u32,
    /// The runs of the entries of finished loads, by load (`units.rs` `Dec`).
    #[serde(skip)]
    pub(crate) run_cache: rustc_hash::FxHashMap<u32, crate::units::Runs>,
}

fn session_owner() -> u32 {
    crate::ids::SESSION
}

// Helpers below are used by the slices being filled in (CONTRIBUTING.md).
#[allow(dead_code)]
/// A use of a TIME unit below one second (LRM 5.2.4.2).
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct TimeUnitUse {
    pub span: Span,
    /// The unit as written, and its size in femtoseconds.
    pub unit: Symbol,
    pub fs: i64,
}

impl Db {
    /// Declarations, expressions and statements from which [`Db::release_analysis`] frees.
    const FREE_FROM: usize = 50_000;

    /// Free the analysis results (units, declarations, expressions, statements, types, scopes
    /// and the analysis caches), keeping only the source map, for a run that needs nothing
    /// else of the design once it is elaborated and lowered (#409). Nothing may use the
    /// design below the source map afterwards: ids into the arenas dangle. The memory is freed
    /// on a thread of its own: dropping every node one by one took a fifth of a short run.
    pub fn release_analysis(&mut self) {
        let size = self.decls.len() + self.exprs.len() + self.stmts.len();
        let mut junk: Vec<Box<dyn std::any::Any + Send>> = Vec::new();
        macro_rules! free {
            ($($f:ident),*) => {$(junk.push(Box::new(std::mem::take(&mut self.$f)));)*};
        }
        free!(
            libs,
            units,
            decls,
            scopes,
            types,
            subtypes,
            exprs,
            stmts,
            consts,
            insts,
            views,
            externals,
            attr_values,
            attr_index,
            forwarding,
            disconnect_times,
            ret_names,
            lookup_cache,
            memo,
            name_memo,
            first_param,
            memo_pool,
            files,
            ext_memo,
            ext_uses,
            pkg_targets,
            written_marks,
            config_specs,
            range_records,
            time_units,
            lib_by_key,
            generic
        );
        // A small design stays allocated until the process ends: a thread and the freeing cost
        // a thirtieth of a tiny run (#426), and the memory is a few megabytes.
        if size < Self::FREE_FROM {
            std::mem::forget(junk);
            return;
        }
        // A failed spawn drops the closure, and with it the memory, here.
        let _ = std::thread::Builder::new().spawn(move || drop(junk));
    }

    /// The recorded uses of a TIME unit smaller than `limit_fs` femtoseconds, in source order
    /// of analysis, each location once.
    pub fn fine_time_units(&self, limit_fs: i64) -> Vec<TimeUnitUse> {
        let mut seen = HashSet::default();
        self.time_units
            .iter()
            .filter(|u| u.fs < limit_fs && seen.insert((u.span.file, u.span.start)))
            .copied()
            .collect()
    }

    /// Record a TIME physical literal of unit `unit` (`fs` femtoseconds) unless its abstract
    /// literal is zero or the unit is a second or more (it is never below a resolution limit).
    pub(crate) fn note_time_unit(
        &mut self,
        ty: TypeId,
        unit: Symbol,
        fs: i64,
        zero: bool,
        span: Span,
    ) {
        // The built-in std and ieee sources are exempt: `std.textio`'s default `UNIT : TIME := ns`
        // would make every design that uses `write` fail under a limit coarser than `ns`.
        let builtin = self.stack.last().is_some_and(|cx| {
            matches!(
                self.libs[cx.lib].source,
                crate::loader::LibSource::StdDir(_)
            )
        });
        if !zero && !builtin && fs < 1_000_000_000_000_000 && self.well_known.time == Some(ty) {
            self.time_units.push(TimeUnitUse { span, unit, fs });
        }
    }

    pub(crate) fn run_time_key(&self) -> (usize, usize, usize) {
        (self.units.len(), self.decls.len(), self.exprs.len())
    }

    /// Packages outside std with objects that exist at run time: signals, shared variables,
    /// files, or constants sem did not fold (in the declaration or the body), or the aliases of
    /// external names its subprograms use (#255), in unit order.
    pub fn run_time_package_set(&self) -> Vec<UnitId> {
        use crate::decl::{DeclKind, ObjClass};
        let run_time = |d: DeclId| match &self.decls[d].kind {
            DeclKind::Object(o) => match o.class {
                ObjClass::Constant => o.default.is_some_and(|e| self.exprs[e].value.is_none()),
                _ => true,
            },
            _ => self.decl_subprograms(d).into_iter().any(|m| {
                !self
                    .external_uses(crate::external::ExtOwner::Sub(m))
                    .is_empty()
            }),
        };
        self.units
            .iter()
            .filter(|(_, u)| self.libs[u.lib].name.key.as_str() != "std")
            .filter(|&(id, _)| {
                self.package_regions(id).is_some_and(|rs| {
                    rs.into_iter()
                        .any(|s| self.decls_with_instances(s).into_iter().any(run_time))
                })
            })
            .map(|(id, _)| id)
            .collect()
    }

    /// [`Db::run_time_package_set`] as stored by [`Db::to_image`], while the design has not
    /// grown since: finding it decodes the declarations of every package.
    pub fn stored_run_time_packages(&self) -> Option<&[UnitId]> {
        match &self.run_time_pkgs {
            Some((key, p)) if *key == self.run_time_key() => Some(p),
            _ => None,
        }
    }

    /// The [`Facts`] of the design as it is now: the stored ones when nothing was added since,
    /// else the stored ones extended by the entries added since (found now).
    pub fn facts(&self) -> std::borrow::Cow<'_, Facts> {
        match &self.facts {
            Some(f) if f.exprs == self.exprs.len() && f.stmts == self.stmts.len() => {
                std::borrow::Cow::Borrowed(f)
            }
            f => std::borrow::Cow::Owned(self.scan_facts(f.as_ref())),
        }
    }

    /// The facts of the design: those of `prev` (found in the first entries of the arenas, which
    /// analysis never changes afterwards) and those of the entries added since, so that a `Db`
    /// restored from an image does not decode the arenas of a prefix again to settle.
    pub(crate) fn scan_facts(&self, prev: Option<&Facts>) -> Facts {
        let mut f = prev.cloned().unwrap_or_default();
        // The entries of units replayed from the analysed-unit cache come with their facts.
        self.scan_new(&self.exprs, f.exprs, |i| {
            self.note_expr(ExprId::from_index(i), &mut f)
        });
        self.scan_new(&self.stmts, f.stmts, |i| {
            self.note_stmt(StmtId::from_index(i), &mut f)
        });
        for u in self.unit_facts.values() {
            f.reads_activity |= u.reads_activity;
            f.forces.extend_from_slice(&u.forces);
            f.package_paths.extend_from_slice(&u.package_paths);
        }
        if !self.unit_facts.is_empty() {
            f.forces.sort_unstable_by_key(|s| s.index());
            f.package_paths.sort_unstable_by_key(|p| p.0.index());
        }
        f.exprs = self.exprs.len();
        f.stmts = self.stmts.len();
        f
    }

    /// Call `f` with the index of every entry of `a` from `from` on that no replayed unit with
    /// recorded facts made.
    fn scan_new<I, T>(&self, a: &Arena<I, T>, from: usize, mut f: impl FnMut(usize)) {
        let runs = a.runs_raw();
        let first = runs
            .partition_point(|r| r.0 as usize <= from)
            .saturating_sub(1);
        for (k, &(start, owner)) in runs.iter().enumerate().skip(first) {
            if self.unit_facts.contains_key(&owner) {
                continue;
            }
            let end = runs.get(k + 1).map_or(a.len(), |r| r.0 as usize);
            for i in (start as usize).max(from)..end {
                f(i);
            }
        }
    }

    /// Add what expression `id` contributes to `f`.
    pub(crate) fn note_expr(&self, id: ExprId, f: &mut Facts) {
        use crate::hir::{ExprKind, PredefAttr};
        match self.exprs[id].kind {
            ExprKind::Attr {
                attr: PredefAttr::Active | PredefAttr::LastActive,
                ..
            } => f.reads_activity = true,
            ExprKind::External(x)
                if self.externals[x].path_kind == yavs_syntax::ast::PathKind::Package =>
            {
                f.package_paths.push((id, x));
            }
            _ => {}
        }
    }

    /// Add what statement `id` contributes to `f`.
    pub(crate) fn note_stmt(&self, id: StmtId, f: &mut Facts) {
        use crate::hir::StmtKind;
        if matches!(
            self.stmts[id].kind,
            StmtKind::Force { .. } | StmtKind::Release { .. }
        ) {
            f.forces.push(id);
        }
    }

    pub fn new(opts: Options) -> Db {
        let mut scopes = Arena::tracked();
        let root = scopes.push(Scope::new(ScopeKind::Root, None, None));
        let mut decls: Arena<DeclId, Decl> = Arena::tracked();
        let mut types: Arena<TypeId, TypeDef> = Arena::new();
        let mut subtypes: Arena<SubtypeId, SubtypeDef> = Arena::tracked();
        let mut exprs: Arena<ExprId, Expr> = Arena::new();
        let name = Ident {
            text: Symbol::intern("<error>"),
            key: Symbol::intern("<error>"),
            span: Span::default(),
        };
        let decl = decls.push(Decl {
            name: Designator::Ident(name),
            kind: DeclKind::Error,
            scope: root,
            span: Span::default(),
            implicit: true,
            since: LangRev::Vhdl2008,
        });
        let ty = types.next_id();
        let subtype = subtypes.push(SubtypeDef {
            base: ty,
            name: Some(name),
            parent: None,
            constraint: Constraint::None,
            resolution: None,
            span: Span::default(),
        });
        types.push(TypeDef {
            decl,
            name,
            anonymous: false,
            kind: TypeKind::Error,
            first_subtype: subtype,
            base_subtype: subtype,
        });
        let expr = exprs.push(Expr {
            kind: ExprKind::Error,
            subtype,
            span: Span::default(),
            stat: Staticness::Locally,
            value: None,
        });
        Db {
            rev: opts.rev,
            opts,
            sources: SourceMap::default(),
            libs: Arena::new(),
            units: Arena::new(),
            decls,
            scopes,
            types,
            subtypes,
            exprs,
            stmts: Arena::new(),
            consts: Arena::new(),
            insts: Arena::new(),
            views: Arena::new(),
            externals: Arena::new(),
            attr_values: Arena::new(),
            attr_index: HashMap::default(),
            forwarding: HashMap::default(),
            disconnect_times: HashMap::default(),
            ret_names: HashMap::default(),
            time_units: Vec::new(),
            attr_closed: HashSet::default(),
            pending_label_attrs: Vec::new(),
            pending_inst_checks: Vec::new(),
            pending_groups: Vec::new(),
            well_known: WellKnown::default(),
            poison: Poison {
                decl,
                ty,
                subtype,
                expr,
            },
            root,
            lib_by_key: HashMap::default(),
            files: HashMap::default(),
            stack: Vec::new(),
            undeclared: HashSet::default(),
            rev_reported: HashSet::default(),
            // A library's worth of lookups fills it; sized up front so it does not rehash
            // a dozen times on the way.
            lookup_cache: HashMap::with_capacity_and_hasher(1 << 13, Default::default()),
            memo: HashMap::default(),
            name_memo: HashMap::default(),
            first_param: Vec::new(),
            memo_pool: Vec::new(),
            pending_implicit: Vec::new(),
            validated: Default::default(),
            overflow: Vec::new(),
            required_ctx: false,
            inst_types: Vec::new(),
            hidden_types: Vec::new(),
            param_anon: Vec::new(),
            anon_reuse: Default::default(),
            ext_memo: HashMap::default(),
            ext_uses: HashMap::default(),
            pkg_targets: HashMap::default(),
            written_marks: HashMap::default(),
            incomplete_ok: false,
            context_subtype: None,
            top_context: None,
            tainted: HashSet::default(),
            stmt_owners: Vec::new(),
            config_specs: HashMap::default(),
            range_records: HashMap::default(),
            pending_specs: Vec::new(),
            generic: Default::default(),
            facts: None,
            run_time_pkgs: None,
            unit_facts: Default::default(),
            diags: Vec::new(),
            errors: 0,
            loads: Vec::new(),
            file_owner: Vec::new(),
            frames: Vec::new(),
            query_log: Default::default(),
            undo: Vec::new(),
            owner: crate::ids::SESSION,
            replaying: 0,
            run_cache: Default::default(),
        }
    }

    // -- diagnostics ---------------------------------------------------------

    /// Record a diagnostic. Every sem diagnostic goes through here.
    pub fn report(&mut self, d: Diagnostic) {
        if d.severity == Severity::Error {
            self.errors += 1;
        }
        self.diags.push(d);
    }

    /// Report an error and return the marker callers propagate.
    pub(crate) fn error(&mut self, d: Diagnostic) -> Reported {
        debug_assert_eq!(d.severity, Severity::Error);
        self.report(d);
        Reported
    }

    /// Internal compiler error at the span being processed (plan: error motto).
    pub(crate) fn ice(&mut self, span: Span, what: impl Into<String>) -> Reported {
        self.error(
            Diagnostic::error(diag::INTERNAL, format!("internal error: {}", what.into()))
                .with_primary(span, "while analyzing this")
                .with_note("this is a bug in yavs")
                .with_help("please report it with the source that triggers it"),
        )
    }

    /// A construct a later slice implements (sem.md section 10). Never a silent
    /// default: analysis of the enclosing unit fails.
    pub(crate) fn unsupported(&mut self, span: Span, what: &str, slice: &str) -> Reported {
        self.error(
            Diagnostic::error(
                diag::INTERNAL,
                format!("{what} is not implemented yet in yavs's semantic analysis"),
            )
            .with_primary(span, "not supported yet")
            .with_note(format!(
                "planned for slice {slice} of docs/design/sem.md section 10; this is a limitation of yavs, not an error in the design"
            )),
        )
    }

    /// The source text of `span`, as written, for messages.
    pub(crate) fn source_text(&self, span: Span) -> String {
        let f = self.sources.get(span.file);
        f.text()[f.text_offset(span.start)..f.text_offset(span.end)].to_string()
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diags
    }

    pub fn take_diagnostics(&mut self) -> Vec<Diagnostic> {
        std::mem::take(&mut self.diags)
    }

    pub fn error_count(&self) -> usize {
        self.errors
    }

    // -- arena helpers -------------------------------------------------------

    pub(crate) fn add_scope(
        &mut self,
        kind: ScopeKind,
        parent: Option<ScopeId>,
        owner: Option<DeclId>,
    ) -> ScopeId {
        self.scopes.push(Scope::new(kind, parent, owner))
    }

    pub(crate) fn add_const(&mut self, v: ConstValue) -> ConstId {
        self.consts.push(v)
    }

    pub(crate) fn add_expr(&mut self, e: Expr) -> ExprId {
        self.exprs.push(e)
    }

    /// The type of a subtype.
    pub fn base(&self, st: SubtypeId) -> TypeId {
        self.subtypes[st].base
    }

    /// The folded value of an expression, if any.
    pub fn value(&self, e: ExprId) -> Option<&ConstValue> {
        self.exprs[e].value.map(|c| &self.consts[c])
    }

    pub(crate) fn cur(&self) -> Option<UnitCx> {
        self.stack.last().copied()
    }

    /// The revision of the file being analyzed (`F`), or `D` outside any unit.
    pub(crate) fn file_rev(&self) -> LangRev {
        self.cur().map_or(self.rev, |c| c.file_rev)
    }
}
