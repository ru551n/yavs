//! The session database: arenas, well-known entities, diagnostics (sem.md 1.1).
//!
//! One `Db` per session. Ids are deterministic for a given input because
//! analysis order is deterministic; they are not stable across inputs.

use crate::tracked::{TrackedMap, TrackedSet};
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
    /// `'last_event` or `'last_value` (of more than one element) of something other than a
    /// whole signal object: a signal parameter, an external name, a non-name (#519).
    pub history_all: bool,
    /// The signal objects of which a part (an element, slice or field, or a non-static selection)
    /// is the prefix of such an attribute.
    pub history_parts: Vec<DeclId>,
    /// The signal objects that are whole such a prefix (the elaborator checks they are whole
    /// kernel signals).
    pub history_wholes: Vec<DeclId>,
    /// The signal objects of which one element is read (a scalar index, selection or alias of a
    /// part, or a scalar port read whole, which is an element of its actual when the port is
    /// collapsed into it; in any expression that is not an assignment target, a port-map actual
    /// or a sensitivity entry): the kernel keeps byte codes for those (#734). Found from the
    /// `elem_direct`, `param_reads` and `pass_edges` of the whole design (`Db::settle_elem_reads`).
    pub elem_reads: Vec<DeclId>,
    /// An element of something the elaborator cannot trace to signal objects is read (an external
    /// name, a signal passed to a subprogram it cannot see into): any signal may be it.
    pub elem_reads_all: bool,
    /// The signal objects with a direct element read, the signal parameters with one, the
    /// subprogram calls with a signal actual `(callee, formal, root object of the actual)` (the
    /// root is `None` for an external name), and an element of an external name read
    /// directly: what `elem_reads` and `elem_reads_all` are found from.
    pub(crate) elem_direct: Vec<DeclId>,
    pub(crate) param_reads: Vec<DeclId>,
    pub(crate) pass_edges: Vec<(DeclId, u16, Option<DeclId>)>,
    pub(crate) elem_ext: bool,
    /// While scanning: the scalar names with a signal root `(expression, span, root, whether the
    /// root is a signal parameter)`, the expressions that are no reads (assignment targets and
    /// plain port-map actuals) and the spans of sensitivity entries, which settle the candidates
    /// into `elem_direct` and `param_reads` (`Facts::settle_elem_cands`).
    #[serde(skip)]
    elem_cands: Vec<(ExprId, Span, DeclId, bool)>,
    #[serde(skip)]
    targets: Vec<ExprId>,
    #[serde(skip)]
    sens_spans: Vec<Span>,
    /// The expressions that were package-path external names (`@lib.pkg.obj`) with their names.
    pub(crate) package_paths: Vec<(ExprId, ExternalId)>,
}

impl Facts {
    /// Turn the candidates found since the last call into `elem_direct` and `param_reads`: the
    /// names of a scalar part that are read, not assigned, connected or listed.
    pub(crate) fn settle_elem_cands(&mut self) {
        let targets: HashSet<ExprId> = std::mem::take(&mut self.targets).into_iter().collect();
        let sens: HashSet<Span> = std::mem::take(&mut self.sens_spans).into_iter().collect();
        for (e, span, d, param) in std::mem::take(&mut self.elem_cands) {
            if !targets.contains(&e) && !sens.contains(&span) {
                if param {
                    self.param_reads.push(d);
                } else {
                    self.elem_direct.push(d);
                }
            }
        }
    }
}

/// A cached `lookup`: revision, epoch, generation of the key, direct hit, result.
pub(crate) type LookupAnswer = (LangRev, u64, u32, bool, crate::scope::Visible);

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
    pub attr_index: TrackedMap<(DeclId, Symbol), AttrValueId>,
    /// 2019 method aliases `alias m is v.m;`: the alias to the object `v` (a private
    /// variable, a shared variable, a variable) whose method it forwards (5.6.2, 6.6.3).
    pub(crate) forwarding: TrackedMap<DeclId, DeclId>,
    /// Disconnection specifications (7.4): the time expression of each guarded signal that has
    /// one (a guarded signal without an entry disconnects after 0 fs).
    pub disconnect_times: TrackedMap<DeclId, ExprId>,
    /// 2019 named return values (`return r of t`, 4.2.1), by the function's parameter region:
    /// the hidden constant whose descriptor is the call's result subtype (M2 passes it after
    /// the parameters) and the subtype `r` it constrains.
    pub ret_names: TrackedMap<ScopeId, (DeclId, SubtypeId)>,
    /// Every non-zero TIME physical literal or unit name below one second that was analyzed,
    /// for the resolution limit check at elaboration (LRM 5.2.4.2, [`Db::fine_time_units`]).
    pub time_units: Vec<TimeUnitUse>,
    /// (region, attribute key, entity class) closed by `others`/`all` (7.2).
    pub(crate) attr_closed: TrackedSet<(ScopeId, Symbol, yavs_syntax::token::Kw)>,
    /// Specifications of labels of statements not analyzed yet (attr.rs, 7.2).
    #[serde(skip)]
    pub(crate) pending_label_attrs: Vec<crate::attr::PendingLabel>,
    /// References in a VHDL-1993 file to an `out` port or parameter of the unit being analyzed
    /// that no write context has claimed yet; the rest are reads, `E1360` at the end of the
    /// unit (LRM 1993 4.3.2, `Db::not_read`, `Db::report_out_reads`).
    #[serde(skip)]
    pub(crate) out_reads: Vec<ExprId>,
    /// Package instances in a process or subprogram whose generic's body is not declared yet
    /// (instance, region, span): `E1237` is checked when the body arrives.
    #[serde(skip)]
    pub(crate) pending_inst_checks: Vec<(crate::ids::InstId, ScopeId, yavs_diag::Span)>,
    /// The units of the file being analyzed that come after the one being analyzed: a unit may
    /// not need them (13.5, units of a file are analyzed in textual order; #810).
    #[serde(skip)]
    pub(crate) later_in_file: Vec<(LibId, crate::loader::UnitKey)>,
    /// The units declared more than once in the file being analyzed: each is analyzed in turn,
    /// the last replaces the others (13.5), and none goes through the analysed-unit cache, whose
    /// entries cannot tell the copies apart.
    #[serde(skip)]
    pub(crate) dup_in_file: Vec<(LibId, crate::loader::UnitKey)>,
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
    pub(crate) undeclared: TrackedSet<(Option<UnitId>, Symbol)>,
    /// Spans already given `E1007` or `W1008` by rev.rs (once per construct: names are
    /// classified once per resolution pass).
    pub(crate) rev_reported: TrackedSet<Span>,
    /// Bottom-up candidate sets of the complete context being resolved, by AST node address.
    /// `lookup` results valid while `Db::cache_epoch` is unchanged: (scope, key) -> (revision, epoch, generation of the key, direct hit, result).
    #[serde(skip)]
    pub(crate) lookup_cache: HashMap<(ScopeId, Symbol), LookupAnswer>,
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
    pub(crate) tainted: TrackedSet<UnitId>,
    /// The subtype the next complete context's top expression is resolved for
    /// (an object's declared subtype), so an aggregate with `others` knows its
    /// bounds (9.3.3.3). Consumed by `resolve`.
    #[serde(skip)]
    pub(crate) context_subtype: Option<SubtypeId>,
    /// Under `--relaxed` (R5): the run-time subtype of an unconstrained assignment target
    /// whose right-hand side is being resolved, so `others` aggregates take its bounds.
    #[serde(skip)]
    pub(crate) relaxed_target: Option<SubtypeId>,
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
    pub(crate) ext_uses: TrackedMap<crate::external::ExtOwner, Box<[DeclId]>>,
    /// The package object each resolved package-path external name denotes, made by
    /// [`Db::bind_package_paths`] (#255).
    pub(crate) pkg_targets: TrackedMap<ExternalId, DeclId>,
    /// A type mark may name an incomplete type: the designated subtype of an
    /// access type definition is being analyzed (alloc.rs, 5.4.2).
    #[serde(skip)]
    pub(crate) incomplete_ok: bool,
    /// The default expression of an interface object is being analyzed: it may name a
    /// deferred constant before its full declaration (6.4.2.2, `Db::check_deferred_read`).
    #[serde(skip)]
    pub(crate) interface_default: bool,
    /// Implicit operations waiting for a well-known type of `std.standard` (BOOLEAN, STRING, ...).
    #[serde(skip)]
    pub(crate) pending_implicit: Vec<crate::implicit::Pending>,
    /// How much of the arenas `validate` already checked.
    pub(crate) validated: crate::validate::Mark,
    /// Processes and subprograms whose statements are being analyzed, innermost last (stmt.rs).
    #[serde(skip)]
    pub(crate) stmt_owners: Vec<crate::stmt::Owner>,
    /// Configuration specifications (7.3.1) by the declarative region they are in (unit.rs).
    pub config_specs: TrackedMap<ScopeId, Vec<crate::hir::ComponentConfig>>,
    /// Configuration specifications whose instance labels wait for the statement part.
    #[serde(skip)]
    pub(crate) pending_specs: Vec<crate::unit::PendingSpec>,
    /// 2019 range record types (`R'RECORD`) by scalar base type, made on first use (attr.rs).
    pub(crate) range_records: TrackedMap<TypeId, SubtypeId>,
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
    /// The entries read from the store in this session (`units.rs` `EntryCache`).
    #[serde(skip)]
    pub(crate) entry_cache: crate::units::EntryCache,
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
            config_specs,
            range_records,
            time_units,
            lib_by_key,
            entry_cache,
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
            DeclKind::Type(_) | DeclKind::Subtype(_)
                if self
                    .declared_ranges(d)
                    .into_iter()
                    .any(|r| self.dynamic_range(r).is_some()) =>
            {
                true
            }
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

    /// The ranges type or subtype declaration `d` introduces, in declaration order: index,
    /// element, field and scalar ranges, which its elaboration evaluates (LRM 14.4.2.1). Ranges
    /// merged in from a type mark (`subtype s2 is s`, `array (0 to 3) of s`) are the type
    /// mark's declaration's. Empty for other kinds of declarations.
    pub fn declared_ranges(&self, d: DeclId) -> Vec<&crate::types::RangeC> {
        use crate::types::RangeC;
        let (mut all, mut inherited) = (Vec::new(), Vec::new());
        // The ranges of `st` not merged in from the nearest named subtype it derives from
        // (all of them for an element of a named subtype).
        let mut part = |st: SubtypeId, own: bool| {
            let sd = &self.subtypes[st];
            all.extend(self.subtype_ranges(st));
            let mut mark = if own || sd.name.is_none() {
                sd.parent
            } else {
                Some(st)
            };
            while let Some(m) = mark.filter(|&m| self.subtypes[m].name.is_none()) {
                mark = self.subtypes[m].parent;
            }
            if let Some(m) = mark {
                inherited.extend(self.subtype_ranges(m));
            }
        };
        match &self.decls[d].kind {
            DeclKind::Subtype(st) => part(*st, true),
            DeclKind::Type(t) => {
                part(self.types[*t].first_subtype, true);
                match &self.types[*t].kind {
                    TypeKind::Array { elem, .. } => part(*elem, false),
                    TypeKind::Record { elems } => elems.iter().for_each(|e| part(e.subtype, false)),
                    _ => {}
                }
            }
            _ => {}
        }
        let key = |r: &RangeC| match *r {
            RangeC::Explicit { left, .. } => Some(left),
            RangeC::Attr { .. } => None,
        };
        let mut seen: HashSet<ExprId> = inherited.iter().filter_map(|r| key(r)).collect();
        all.retain(|r| key(r).is_none_or(|l| seen.insert(l)));
        all
    }

    /// Every range of the constraint of subtype `st`: index, element, field and scalar ranges.
    pub fn subtype_ranges(&self, st: SubtypeId) -> Vec<&crate::types::RangeC> {
        fn collect<'c>(c: &'c Constraint, out: &mut Vec<&'c crate::types::RangeC>) {
            match c {
                Constraint::Range(rc) => out.push(rc),
                Constraint::Array { dims, elem } => {
                    out.extend(dims.iter().flatten());
                    collect(elem, out);
                }
                Constraint::Record { elems } => elems.iter().for_each(|(_, c)| collect(c, out)),
                Constraint::None | Constraint::FromObject { .. } => {}
            }
        }
        let mut out = Vec::new();
        collect(&self.subtypes[st].constraint, &mut out);
        out
    }

    /// The left bound of `rc` when it is an explicit range with a bound that is not globally
    /// static (an impure call, a shared variable): one a declared type or subtype of a design
    /// unit or package evaluates once, at its declaration, for every later use (#612).
    pub fn dynamic_range(&self, rc: &crate::types::RangeC) -> Option<ExprId> {
        match *rc {
            crate::types::RangeC::Explicit { left, right, .. }
                if self.exprs[left].stat < Staticness::Globally
                    || self.exprs[right].stat < Staticness::Globally =>
            {
                Some(left)
            }
            _ => None,
        }
    }

    /// [`Db::run_time_package_set`] as stored by [`Db::to_image`], while the design has not
    /// grown since and has no external name: finding it decodes the declarations of every
    /// package. With external names the set changes when elaboration binds them (the aliases
    /// of a subprogram's external names are found without growing the design), so it is found
    /// again.
    pub fn stored_run_time_packages(&self) -> Option<&[UnitId]> {
        match &self.run_time_pkgs {
            Some((key, p)) if *key == self.run_time_key() && self.externals.is_empty() => Some(p),
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
            f.history_all |= u.history_all;
            f.history_parts.extend_from_slice(&u.history_parts);
            f.history_wholes.extend_from_slice(&u.history_wholes);
            f.elem_direct.extend_from_slice(&u.elem_direct);
            f.param_reads.extend_from_slice(&u.param_reads);
            f.pass_edges.extend_from_slice(&u.pass_edges);
            f.elem_ext |= u.elem_ext;
            f.package_paths.extend_from_slice(&u.package_paths);
        }
        if !self.unit_facts.is_empty() {
            f.forces.sort_unstable_by_key(|s| s.index());
            f.package_paths.sort_unstable_by_key(|p| p.0.index());
        }
        f.settle_elem_cands();
        for v in [
            &mut f.history_parts,
            &mut f.history_wholes,
            &mut f.elem_direct,
            &mut f.param_reads,
        ] {
            v.sort_unstable_by_key(|d| d.index());
            v.dedup();
        }
        f.pass_edges
            .sort_unstable_by_key(|e| (e.0.index(), e.1, e.2.map(|d| d.index())));
        f.pass_edges.dedup();
        self.settle_elem_reads(&mut f);
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
        use crate::hir::{AttrPrefix, ExprKind, PredefAttr};
        match self.exprs[id].kind {
            ExprKind::Attr {
                attr: PredefAttr::Active | PredefAttr::LastActive,
                ..
            } => f.reads_activity = true,
            ExprKind::Attr {
                attr: attr @ (PredefAttr::LastEvent | PredefAttr::LastValue),
                prefix: AttrPrefix::Expr(p),
                ..
            } => self.note_history(p, matches!(attr, PredefAttr::LastValue), f),
            ExprKind::External(x)
                if self.externals[x].path_kind == yavs_syntax::ast::PathKind::Package =>
            {
                f.package_paths.push((id, x));
            }
            ExprKind::Index { .. } | ExprKind::Field { .. } => self.note_elem_read(id, f),
            ExprKind::Obj(d) if matches!(self.decls[d].kind, DeclKind::Alias(_)) => {
                self.note_elem_read(id, f)
            }
            ExprKind::Obj(d) if self.is_signal_port(d) => self.note_elem_read(id, f),
            ExprKind::Call {
                callee, ref args, ..
            } => self.note_call(callee, args, f),
            _ => {}
        }
    }

    fn is_signal_port(&self, d: DeclId) -> bool {
        use crate::decl::{ObjClass, ObjRole};
        matches!(&self.decls[d].kind, DeclKind::Object(o) if o.class == ObjClass::Signal && o.role == ObjRole::Port)
    }

    /// Add `e` to the candidates of `Facts::elem_reads` if it is a scalar part of a signal, or a
    /// scalar port read whole (#734). A whole scalar signal that is no port is one element
    /// anyway and not noted. A scalar signal parameter read whole is not noted either: its
    /// actual, if a part, is itself a read where the call is.
    fn note_elem_read(&self, e: ExprId, f: &mut Facts) {
        use crate::decl::{ObjClass, ObjRole};
        use crate::hir::ObjRoot;
        if !self.types[self.base(self.exprs[e].subtype)]
            .kind
            .is_scalar()
        {
            return;
        }
        match self.lsp_parts(e) {
            Some((ObjRoot::Decl(d), path, _, cut)) => {
                if let DeclKind::Object(o) = &self.decls[d].kind
                    && o.class == ObjClass::Signal
                    && (!path.is_empty() || cut || o.role == ObjRole::Port)
                {
                    let span = self.exprs[e].span;
                    f.elem_cands.push((e, span, d, o.role == ObjRole::Param));
                }
            }
            Some((ObjRoot::External(x), path, _, cut))
                if self.externals[x].class == yavs_syntax::ast::ExternalClass::Signal
                    && (!path.is_empty() || cut) =>
            {
                f.elem_ext = true
            }
            _ => {}
        }
    }

    /// Add the signal actuals of a call to `Facts::pass_edges`: a callee that reads an element
    /// of its signal parameter reads one of the signal the actual names.
    fn note_call(&self, callee: DeclId, args: &[crate::hir::Arg], f: &mut Facts) {
        use crate::decl::ObjClass;
        use crate::hir::{Actual, ObjRoot};
        let Some(sp) = self.decls[callee].kind.subprogram() else {
            return;
        };
        for a in args {
            if sp
                .params
                .get(a.formal as usize)
                .is_none_or(|p| p.class != ObjClass::Signal)
            {
                continue;
            }
            let actuals: Vec<ExprId> = match &a.actual {
                Actual::Expr(e) => vec![*e],
                Actual::Converted { actual, .. } => vec![*actual],
                Actual::Individual(parts) => parts.iter().map(|p| p.1).collect(),
                _ => continue,
            };
            for e in actuals {
                match self.lsp_parts(e) {
                    Some((ObjRoot::Decl(d), ..)) if matches!(&self.decls[d].kind, DeclKind::Object(o) if o.class == ObjClass::Signal) => {
                        f.pass_edges.push((callee, a.formal, Some(d)))
                    }
                    Some((ObjRoot::External(x), ..))
                        if self.externals[x].class == yavs_syntax::ast::ExternalClass::Signal =>
                    {
                        f.pass_edges.push((callee, a.formal, None))
                    }
                    _ => {}
                }
            }
        }
    }

    /// Find `Facts::elem_reads` and `elem_reads_all` from the raw facts: a signal passed to a
    /// subprogram that reads an element of that signal parameter (directly or by passing it on)
    /// has an element read. A callee the elaborator cannot see into asks everything.
    pub(crate) fn settle_elem_reads(&self, f: &mut Facts) {
        use crate::decl::{Body, ObjRole};
        // A parameter is its body region and its position among the region's parameters, the
        // same for the declaration a call names and for the body's own parameter objects.
        let key = |p: DeclId| -> Option<(ScopeId, u16)> {
            let sc = self.decls[p].scope;
            let is_param = |d: &&DeclId| matches!(&self.decls[**d].kind, DeclKind::Object(o) if o.role == ObjRole::Param);
            let pos = self.scopes[sc]
                .order
                .iter()
                .filter(is_param)
                .position(|&d| d == p)?;
            Some((sc, pos as u16))
        };
        let mut all = f.elem_ext;
        let mut reads: HashSet<(ScopeId, u16)> = HashSet::default();
        for &p in &f.param_reads {
            match key(p) {
                Some(k) => {
                    reads.insert(k);
                }
                None => all = true,
            }
        }
        enum To {
            Obj(DeclId),
            Param((ScopeId, u16)),
            External,
        }
        let mut edges = Vec::new();
        for &(callee, formal, actual) in &f.pass_edges {
            let from = match self.decls[callee].kind.subprogram().map(|s| &s.body) {
                Some(Body::Hir { scope, .. }) => (*scope, formal),
                Some(Body::Native(_) | Body::Implicit(_)) => continue,
                _ => {
                    all = true;
                    continue;
                }
            };
            let to = match actual {
                None => To::External,
                Some(d) if self.is_param_decl(d) => match key(d) {
                    Some(k) => To::Param(k),
                    None => {
                        all = true;
                        continue;
                    }
                },
                Some(d) => To::Obj(d),
            };
            edges.push((from, to));
        }
        let mut objs: HashSet<DeclId> = f.elem_direct.iter().copied().collect();
        loop {
            let mut changed = false;
            for (from, to) in &edges {
                if reads.contains(from) {
                    changed |= match to {
                        To::Obj(d) => objs.insert(*d),
                        To::Param(k) => reads.insert(*k),
                        To::External => !std::mem::replace(&mut all, true),
                    };
                }
            }
            if !changed {
                break;
            }
        }
        f.elem_reads = objs.into_iter().collect();
        f.elem_reads.sort_unstable_by_key(|d| d.index());
        f.elem_reads_all = all;
    }

    fn is_param_decl(&self, d: DeclId) -> bool {
        use crate::decl::ObjRole;
        matches!(&self.decls[d].kind, DeclKind::Object(o) if o.role == ObjRole::Param)
    }

    /// Add what `p'last_event` (or `p'last_value`) contributes to `f`: what the kernel must keep
    /// of the event history of the signal `p` names. `'last_value` of a scalar is the value
    /// before the element's own last change, which needs no history.
    fn note_history(&self, p: ExprId, last_value: bool, f: &mut Facts) {
        use crate::hir::ObjRoot;
        if last_value
            && self.types[self.base(self.exprs[p].subtype)]
                .kind
                .is_scalar()
        {
            return;
        }
        match self.lsp_parts(p) {
            Some((ObjRoot::ImplicitSignal(_), ..)) => {}
            Some((ObjRoot::Decl(d), path, _, cut)) if !self.is_param(ObjRoot::Decl(d)) => {
                if path.is_empty() && !cut {
                    f.history_wholes.push(d)
                } else {
                    f.history_parts.push(d)
                }
            }
            _ => f.history_all = true,
        }
    }

    /// Add what statement `id` contributes to `f`.
    pub(crate) fn note_stmt(&self, id: StmtId, f: &mut Facts) {
        use crate::hir::StmtKind;
        match &self.stmts[id].kind {
            StmtKind::Force { target, .. } | StmtKind::Release { target, .. } => {
                f.forces.push(id);
                f.targets.push(target.expr);
            }
            StmtKind::SignalAssign { target, .. } | StmtKind::ConcSignalAssign { target, .. } => {
                f.targets.push(target.expr)
            }
            // Connections and sensitivity entries are no reads (#734).
            StmtKind::Instance { port_map, .. } | StmtKind::Block { port_map, .. } => {
                for a in port_map.iter() {
                    if let crate::hir::Actual::Expr(e) = a.actual
                        && !a.inertial
                    {
                        f.targets.push(e);
                    }
                }
            }
            StmtKind::Process {
                sensitivity: crate::hir::Sensitivity::List(ls),
                ..
            } => f.sens_spans.extend(ls.iter().map(|l| l.span)),
            StmtKind::Wait {
                on, until: None, ..
            } => f.sens_spans.extend(on.iter().map(|l| l.span)),
            StmtKind::ProcCall(c) | StmtKind::ConcProcCall { call: c, .. } => {
                self.note_call(c.callee, &c.args, f)
            }
            _ => {}
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
            since: LangRev::Vhdl1993,
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
            attr_index: Default::default(),
            forwarding: Default::default(),
            disconnect_times: Default::default(),
            ret_names: Default::default(),
            time_units: Vec::new(),
            attr_closed: Default::default(),
            pending_label_attrs: Vec::new(),
            out_reads: Vec::new(),
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
            undeclared: Default::default(),
            rev_reported: Default::default(),
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
            ext_uses: Default::default(),
            pkg_targets: Default::default(),
            incomplete_ok: false,
            interface_default: false,
            later_in_file: Vec::new(),
            dup_in_file: Vec::new(),
            context_subtype: None,
            relaxed_target: None,
            top_context: None,
            tainted: Default::default(),
            stmt_owners: Vec::new(),
            config_specs: Default::default(),
            range_records: Default::default(),
            pending_specs: Vec::new(),
            generic: Default::default(),
            facts: None,
            run_time_pkgs: None,
            unit_facts: Default::default(),
            diags: Vec::new(),
            errors: 0,
            loads: Vec::new(),
            entry_cache: Default::default(),
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
                diag::NOT_IMPLEMENTED,
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
