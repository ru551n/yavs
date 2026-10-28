//! Elaboration (docs/design/ir.md section 4): an analyzed design (yavs-sem HIR) becomes a
//! hierarchy of scopes, a signal net graph and a list of process instances, which
//! [`Elaborated::build_with`] turns into a kernel model.
//!
//! [`elaborate`] is a pure function of (design, top, generics, configuration): it keeps no
//! global state, so it can run again in the same process (rule 4, sim2 #1184), and building
//! the kernel model from one [`Elaborated`] can be repeated for an in-process restart.
//!
//! Fail early: any error-severity diagnostic means there is no [`Elaborated`].

mod build;
mod callcache;
mod collapse;
mod eval;
mod hier;
mod keep_codes;
mod lean;
pub mod libstage;
mod pkgcall;
mod sim;
mod stuck;
mod value;
mod walk;
mod whole;

#[cfg(test)]
mod tests;

pub use callcache::stats as call_cache_stats;
pub use eval::{Class, ConvFn, Enc, Model, ScopeVals, Side, Val, elem_of};
pub use hier::{
    Bounds, Case, DriverDecl, Hierarchy, ImplicitKind, Leaf, LeafId, LeafLoc, LeafRes, NetGraph,
    ObjId, ObjKind, PortEdge, ProcIdx, Scope, ScopeId, ScopeKind, Sel, Shape, SigObj,
};
pub use sim::{Built, Ir};
pub use stuck::{ProcTable, StuckWhy};
/// Values in IR memory and back (VHPI variables, #309).
pub use value::{CellKind, DataLeaf, decode as ir_decode, encode as ir_encode, leaf_locs};
pub use walk::{load_bodies, load_bodies_in};
pub use yavs_interp::elaboration_reports;
pub use yavs_lower::LowerOptions;

use yavs_diag::Diagnostic;
use yavs_sem::{Db, StmtId, UnitId};

/// Diagnostic codes of elaboration (`E2100`-`E2199`, `W2100`-`W2199`).
pub mod codes {
    /// The top-level unit named on the command line does not exist or is not elaboratable.
    pub const TOP_NOT_FOUND: &str = "E2101";
    /// An entity has no architecture to elaborate.
    pub const NO_ARCHITECTURE: &str = "E2102";
    /// A generic has no value: no actual, no override, no default.
    pub const GENERIC_NO_VALUE: &str = "E2103";
    /// A `-gNAME=VALUE` text is not a value of the generic's type.
    pub const BAD_OVERRIDE: &str = "E2104";
    /// `-gNAME=VALUE` for a generic the top level does not have.
    pub const UNKNOWN_OVERRIDE: &str = "W2105";
    /// An expression that has to be evaluated during elaboration cannot be.
    pub const NOT_EVALUATED: &str = "E2106";
    /// Evaluation during elaboration fails (overflow, division by zero, out of range).
    pub const EVAL_FAILED: &str = "E2107";
    /// A component instance is not bound to an entity (LRM 7.3.3): it stays open.
    pub const UNBOUND: &str = "W2108";
    /// Instances nested deeper than the limit (recursive instantiation).
    pub const TOO_DEEP: &str = "E2109";
    /// A port actual does not match its formal (shape, length).
    pub const PORT_MISMATCH: &str = "E2110";
    /// A port of an unconstrained subtype has nothing to take its bounds from.
    pub const PORT_UNCONSTRAINED: &str = "E2111";
    /// Several sources for an unresolved signal (LRM 14.7.2).
    pub const MULTIPLE_SOURCES: &str = "E2112";
    /// A static name selects elements outside its prefix.
    pub const BAD_NAME: &str = "E2113";
    /// A subprogram is called before its body is elaborated (LRM 14.4.2).
    pub const BODY_NOT_ELABORATED: &str = "E2114";
    /// A generic value is outside the generic's subtype.
    pub const GENERIC_RANGE: &str = "E2115";
    /// A package needs a body (subprograms, deferred constants) and has none (LRM 4.8).
    pub const BODY_MISSING: &str = "E2116";
    /// An external name names no elaborated object (LRM 8.7, 14.4.1).
    pub const EXTERNAL_NOT_FOUND: &str = "E2117";
    /// A constant with an impure initial value elaboration cannot evaluate yet runs its
    /// initial value when the simulation starts, after the elaboration-time calls (#220).
    pub const IMPURE_AT_RUN_TIME: &str = "W2118";
    /// A top-level variable port (2019) has no actual (LRM 6.5.6.3, #188).
    pub const VARIABLE_PORT_OPEN: &str = "E2119";
    /// A TIME unit below the resolution limit appears in the design hierarchy (LRM 5.2.4.2).
    pub const TIME_BELOW_RESOLUTION: &str = "E2120";
    /// An external name's subtype indication does not match the object it names (LRM 8.7).
    pub const EXTERNAL_MISMATCH: &str = "E2121";
    /// Not implemented yet in yavs (names the slice or milestone).
    pub const NOT_IMPLEMENTED: &str = "E9999";
    /// Internal error: a broken invariant ("this is a bug in yavs").
    pub const INTERNAL: &str = "E9999";
}

/// LRM 5.2.4.2: it is an error if a unit of TIME smaller than the resolution limit appears in
/// the elaborated design hierarchy, unless its abstract literal is zero. `db` holds exactly the
/// units elaboration loaded (the hierarchy and the packages it uses, std and ieee included when
/// they contain such a literal), whether or not the code runs. The rule is about the unit, not
/// the value (`0.4 ns` is legal under `ns`).
pub fn check_time_resolution(db: &Db, limit: yavs_kernel::Unit) -> Option<Diagnostic> {
    /// Locations listed in the diagnostic; the rest is counted.
    const LISTED: usize = 10;
    let uses = db.fine_time_units(limit.fs() as i64);
    let first = uses.first()?;
    let mut d = Diagnostic::error(
        codes::TIME_BELOW_RESOLUTION,
        format!(
            "{} TIME literal{} with a unit below the resolution limit of {} in the design",
            uses.len(),
            if uses.len() == 1 { "" } else { "s" },
            format!("{limit:?}").to_lowercase(),
        ),
    )
    .with_primary(
        first.span,
        format!("unit `{}` is below the resolution limit", first.unit),
    );
    for u in &uses[1..uses.len().min(LISTED)] {
        d = d.with_secondary(u.span, format!("unit `{}`", u.unit));
    }
    if uses.len() > LISTED {
        d = d.with_note(format!("and {} more", uses.len() - LISTED));
    }
    Some(d.with_note("LRM 5.2.4.2: this holds for every TIME literal in the hierarchy, in packages it uses and in code that never runs; only a literal whose abstract literal is zero is exempt")
        .with_help("select a finer limit with `--resolution`, or use a coarser unit"))
}

/// Elaboration settings.
#[derive(Clone, Debug)]
pub struct ElabConfig {
    /// Instances nested deeper than this are an error (recursive designs end at a generate).
    pub max_depth: u32,
    /// Collapse `in` ports whose actual is whole signal leaves into aliases of the actual
    /// (`collapse.rs`, plan rule 3): on by default, off with `--no-collapse-ports`.
    pub collapse_ports: bool,
    /// Store runs of record leaves in one kernel signal (`NetGraph::place`, #686): on by
    /// default; off (one signal per leaf) with the hidden test aid `YAVS_PACKS=off`.
    pub packs: bool,
}

impl Default for ElabConfig {
    fn default() -> Self {
        ElabConfig {
            max_depth: 1000,
            collapse_ports: true,
            packs: true,
        }
    }
}

/// What `yavs elab TOP` names: an entity, `entity.arch` or a configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Top {
    pub entity: UnitId,
    pub arch: UnitId,
    pub config: Option<UnitId>,
}

/// One process instance, in elaboration order (= kernel `ProcId` order).
pub struct ProcInst {
    /// Its scope (kind [`ScopeKind::Process`]); the parent is the enclosing region.
    pub scope: ScopeId,
    /// The process or concurrent statement.
    pub stmt: StmtId,
    pub postponed: bool,
    /// Indices into `NetGraph::drivers` owned by this process (merged per leaf).
    pub drivers: Vec<usize>,
    /// For each longest static prefix of the statement's `ProcessInfo::drivers`, in order:
    /// the driver indices and kernel ranges it covers.
    pub lsps: Vec<Vec<(usize, LeafId, yavs_kernel::Range)>>,
    /// The equivalent process of a port's expression actual (LRM 6.5.6.3).
    pub actual: Option<Box<PortActual>>,
    /// A clocked process on one edge of one scalar signal
    /// (`yavs_kernel::EdgeTrigger`, `Walker::edge_trigger`).
    pub trigger: Option<ProcTrigger>,
    /// A concurrent `target <= source;` of a static signal name (`Walker::copy_source`): the
    /// source parts, one per part of `lsps[0]` (the target) and of the same length and kind.
    /// The kernel copies them itself (`yavs_kernel::CopyProcess`, #529).
    pub copy: Option<Vec<(LeafId, yavs_kernel::Range)>>,
    /// A concurrent `x <= not x after T;` on one scalar element with `T` known now
    /// (`Walker::toggle_source`): the kernel runs it itself (`yavs_kernel::ToggleProcess`, #440).
    pub toggle: Option<ProcToggle>,
    /// A concurrent `target <= a op b;` (or `not a`) of one logical operator on static signal
    /// parts (`Walker::gate_source`): the kernel runs it itself (`yavs_kernel::GateProcess`, #779).
    pub gate: Option<ProcGate>,
    /// A concurrent `target <= a when c else b;` (`Walker::mux_source`): the kernel runs it
    /// itself (`yavs_kernel::MuxProcess`, #786).
    pub mux: Option<ProcMux>,
    /// A concurrent assignment of an expression tree of logic operators and relations, with any
    /// number of conditional arms (`Walker::prog_source`), when it is not a gate or a mux: the
    /// kernel runs it itself (`yavs_kernel::ProgProcess`, #788).
    pub prog: Option<ProcProg>,
    /// A clocked process (it has a `trigger`) whose activation is a function of the signals
    /// listed here and constants alone (`Db::pure_clocked_reads`): the kernel may skip an
    /// activation when none of them had an event since the last one
    /// (`yavs_kernel::Builder::idle_skip`).
    pub idle: Option<Vec<IdleRead>>,
    /// The `_guard` process of a guarded block: the signal parts its condition reads. The kernel
    /// runs it in the update phase of the cycle in which one of them has an event, so GUARD
    /// changes in that cycle (LRM 11.2; `yavs_kernel::Builder::guard_process`).
    pub guard: Option<Vec<(LeafId, yavs_kernel::Range)>>,
}

/// The clock generator `x <= not x after T;` of [`ProcInst::toggle`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcToggle {
    pub leaf: LeafId,
    pub elem: u32,
    /// The delay in femtoseconds, positive.
    pub delay: i64,
    pub transport: bool,
}

/// What a gate or a mux reads: a signal part or a logic value known at elaboration.
#[derive(Clone, Debug, PartialEq)]
pub enum ProcSrc {
    Sig(LeafId, yavs_kernel::Range),
    Lit(yavs_kernel::LogicVec),
}

/// The gate `target <= a op b;` of [`ProcInst::gate`]: operands of the target's length (of one
/// length for `=` and `/=`, whose target is one BOOLEAN element).
#[derive(Clone, Debug, PartialEq)]
pub struct ProcGate {
    pub gate: yavs_kernel::Gate,
    pub a: ProcSrc,
    /// `a` again for `not`.
    pub b: ProcSrc,
    /// The length of the operands.
    pub len: u32,
    /// The `after` delay in femtoseconds, 0 without one.
    pub delay: i64,
    pub transport: bool,
}

/// The condition of a [`ProcMux`].
#[derive(Clone, Debug, PartialEq)]
pub enum ProcCond {
    /// One logic element is one of a set of codes: bit `c` of the mask is set when code `c`
    /// makes the condition true.
    Mask((LeafId, yavs_kernel::Range), u16),
    /// The predefined `a = b` (`/=` when `ne`) of two logic operands of `len` elements.
    Eq {
        a: ProcSrc,
        b: ProcSrc,
        len: u32,
        ne: bool,
    },
}

/// The mux `target <= a when c else b;` of [`ProcInst::mux`]: the target's parts in the order of
/// `ProcInst::lsps`, each with its source in the two arms.
#[derive(Clone, Debug, PartialEq)]
pub struct ProcMux {
    pub cond: ProcCond,
    /// The sources per arm: `[else, then]`.
    pub srcs: [Vec<ProcSrc>; 2],
    /// The `after` delay of each arm in femtoseconds, 0 without one.
    pub delay: [i64; 2],
    pub transport: bool,
}

/// An operand of a [`ProcNode`]: a signal part or value, or the result of an earlier node.
#[derive(Clone, Debug, PartialEq)]
pub enum ProcOpnd {
    Src(ProcSrc),
    Node(u32),
}

/// One operation of a [`ProcProg`]; the result is a code (one element) or a plane value.
#[derive(Clone, Debug, PartialEq)]
pub enum ProcOp {
    /// A logical operator (`b` is `a` again for `not`) over operands of `len` elements, or `=`
    /// and `/=` (one BOOLEAN element).
    Gate(yavs_kernel::Gate, ProcOpnd, ProcOpnd),
    /// One logic element is one of a set of codes (as [`ProcCond::Mask`]); one BOOLEAN element.
    Mask(LeafId, yavs_kernel::Range, u16),
    /// One element of a signal of an enumeration or integer type holds these bytes (`/=` when
    /// the flag is set); one BOOLEAN element.
    Bytes(LeafId, yavs_kernel::Range, Vec<u8>, bool),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProcNode {
    pub op: ProcOp,
    /// The length of the operands.
    pub len: u32,
}

/// One arm of a [`ProcProg`]: when its condition holds (the last arm has none), the target
/// parts take `vals`.
#[derive(Clone, Debug, PartialEq)]
pub struct ProcArm {
    /// A BOOLEAN element.
    pub cond: Option<ProcOpnd>,
    /// The nodes the condition needs, in order (evaluated only when the arm is reached).
    pub cond_nodes: std::ops::Range<u32>,
    /// One per part of `ProcInst::lsps[0]` (several for a record copy, one otherwise).
    pub vals: Vec<ProcOpnd>,
    pub val_nodes: std::ops::Range<u32>,
    /// The `after` delay in femtoseconds, 0 without one.
    pub delay: i64,
}

/// The concurrent assignment `target <= v1 when c1 else v2 when c2 else ... else vn;` (or a
/// plain `target <= v;`) of [`ProcInst::prog`]: nodes in post order, arms in statement order.
#[derive(Clone, Debug, PartialEq)]
pub struct ProcProg {
    pub nodes: Vec<ProcNode>,
    pub arms: Vec<ProcArm>,
    pub transport: bool,
}

impl ProcProg {
    /// Every operand, for renumbering signal parts.
    pub fn opnds_mut(&mut self) -> impl Iterator<Item = &mut ProcOpnd> {
        let nodes = self.nodes.iter_mut().flat_map(|n| match &mut n.op {
            ProcOp::Gate(_, a, b) => vec![a, b],
            ProcOp::Mask(..) | ProcOp::Bytes(..) => vec![],
        });
        let arms = self
            .arms
            .iter_mut()
            .flat_map(|a| a.cond.iter_mut().chain(&mut a.vals));
        nodes.chain(arms)
    }
}

/// A signal part a pure process reads ([`ProcInst::idle`]) and, when it reads it only under some
/// conditions, those: a disjunction of conjunctions of literals, each of which holds in every
/// activation that reads it.
#[derive(Clone, Debug)]
pub struct IdleRead {
    pub leaf: LeafId,
    pub range: yavs_kernel::Range,
    pub guard: Option<Vec<Vec<IdleLit>>>,
}

/// An element of a logic signal compared with a `std_ulogic` code (`eq`: equal, else not equal).
#[derive(Clone, Copy, Debug)]
pub struct IdleLit {
    pub leaf: LeafId,
    pub elem: u32,
    pub code: u8,
    pub eq: bool,
}

/// The trigger of a clocked process ([`ProcInst::trigger`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcTrigger {
    pub rising: bool,
    /// The clock: its signal and element.
    pub leaf: LeafId,
    pub elem: u32,
    /// Valid only with a single suspension point (the `wait until` form).
    pub wait: bool,
    /// `clk'event and clk = '1'` instead of `rising_edge`.
    pub legacy: bool,
    /// An asynchronous reset condition `r = c`: the signal, the element, the `std_ulogic` code.
    pub reset: Option<(LeafId, u32, u8)>,
}

/// The equivalent process `formal <= expr` of a port's expression actual: its one driver is
/// the formal, or the formal's part of shape `shape`; `formal` is the subtype of the formal,
/// or of the part when the expression is a part's in an individual association.
pub struct PortActual {
    pub expr: yavs_sem::ExprId,
    pub formal: Option<yavs_sem::SubtypeId>,
    /// The port.
    pub port: yavs_sem::DeclId,
    pub shape: Shape,
    /// The range of a scalar formal (or part) whose bounds come from the instance's generics, as
    /// `yavs_lower::PortActual::bounds` takes it.
    pub bounds: Option<[u64; 3]>,
}

/// An instance of a `'FOREIGN "VHPI library model"` architecture.
pub struct ForeignArch {
    pub scope: ScopeId,
    pub library: String,
    pub model: String,
}

/// A process of a foreign architecture's model: its name, the elements it drives, and what
/// learns the driver ids at every build (`Hook::assign` schedules on them).
pub struct ExtProc {
    pub name: String,
    pub drivers: Vec<yavs_kernel::SigRef>,
    pub built: DriversBuilt,
}

/// Called with the driver ids of an [`ExtProc`] when the simulation is built.
pub type DriversBuilt = std::sync::Arc<dyn Fn(&[yavs_kernel::DriverId]) + Send + Sync>;

pub struct Elaborated {
    pub top: Top,
    pub hier: Hierarchy,
    pub nets: NetGraph,
    pub procs: Vec<ProcInst>,
    /// Instances of VHPI foreign architectures, in elaboration order (#330): only their
    /// ports and the entity's declarations and statements are elaborated.
    pub foreign_archs: Vec<ForeignArch>,
    /// Processes a VHPI plugin created for foreign architectures (`vhpi_create`), set by the
    /// caller before building: each owns drivers and runs no code.
    pub ext_procs: Vec<ExtProc>,
    /// Values of generics, generate parameters and constants per scope (for lowering).
    pub model_vals: Vec<ScopeVals>,
    /// Per leaf: nothing asks for the event history of single elements of this wide signal, so
    /// the kernel keeps only each block's last event (`lean.rs`). Clear it to keep the history
    /// of every signal; empty means the same.
    pub lean: Vec<bool>,
    /// Per leaf: no process reads a single element of this signal, so the kernel keeps no byte
    /// codes for it (`keep_codes.rs`). Clear it to keep them for every signal; empty means the same.
    pub no_codes: Vec<bool>,
    /// Signal objects by (scope, declaration).
    pub sigs: std::collections::HashMap<(ScopeId, yavs_sem::DeclId), ObjId>,
    /// The declarative regions (yavs-sem scopes) of each elaborated scope, in elaboration
    /// order: an instance's entity then architecture, a block's or generate's own. Processes
    /// have none (their statement names their scope). VHPI lists declarations from it (#269).
    pub regions: std::collections::HashMap<ScopeId, Vec<yavs_sem::ScopeId>>,
    /// The protected object of each 2019 variable port, by (scope of the formal, formal).
    pub var_ports: std::collections::BTreeMap<(ScopeId, yavs_sem::DeclId), VarObj>,
    /// Aliases of external shared variables by (region, alias): the region and declaration
    /// of the variable, whose object the alias's instance data cell holds (#93).
    pub ext_vars:
        std::collections::HashMap<(ScopeId, yavs_sem::DeclId), (ScopeId, yavs_sem::DeclId)>,
    /// Aliases of external constants bound to a constant only its run-time thunk computes,
    /// by (region, alias): the region and declaration of the constant (#255).
    pub ext_consts:
        std::collections::HashMap<(ScopeId, yavs_sem::DeclId), (ScopeId, yavs_sem::DeclId)>,
    /// Aliases of the external names in package subprograms bound once the hierarchy is
    /// elaborated, with their package's scope, and why the others are not: a subprogram
    /// using one fails when it is called (#255).
    pub pkg_ext: std::collections::HashMap<yavs_sem::DeclId, ScopeId>,
    pub ext_unbound: std::collections::HashMap<yavs_sem::DeclId, String>,
    /// The elaborated shape of each process variable and non-protected shared variable whose
    /// bounds elaboration knows, by declaring region (a process variable's: the process's
    /// enclosing region; `None`: a package's) and declaration. The waveform recorder declares
    /// variables with it, as signals with theirs (#321).
    pub var_shapes: std::collections::HashMap<(Option<ScopeId>, yavs_sem::DeclId), Shape>,
    /// Shared variables by (declaring region, declaration): one object each (#249).
    pub(crate) shared_vars: std::collections::HashSet<(ScopeId, yavs_sem::DeclId)>,
    /// The ranges of types and subtypes elaboration evaluated at their declarations, by
    /// declaring scope and left bound (`UnitIr::ranges`, #612).
    pub(crate) ranges: std::collections::HashMap<(ScopeId, yavs_sem::ExprId), Bounds>,
    /// Warnings of a successful elaboration.
    pub warnings: Vec<Diagnostic>,
    /// Conversion functions of port associations (`PortEdge::to_formal`/`to_actual`).
    pub convs: Vec<eval::ConvFn>,
    /// The IR of the processes, once [`Elaborated::lower`] ran.
    pub ir: Option<Ir>,
    /// Calls that ran with package state during elaboration, in order: replayed when the
    /// simulation is built (#77).
    pub(crate) elab_calls: Vec<pkgcall::Call>,
    /// The package state those calls left, for the simulation to start from (#383).
    pub(crate) start: Option<pkgcall::Start>,
}

/// The protected object a 2019 variable port denotes (LRM 6.5.2, #188): the shared
/// variable at the end of its chain of actuals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VarObj {
    /// Shared variable `decl` of region `region`, whose instance data holds its handle.
    Region {
        region: ScopeId,
        decl: yavs_sem::DeclId,
    },
    /// A shared variable of a package.
    Package(yavs_sem::DeclId),
}

impl Elaborated {
    /// The protected object of variable port `d`, seen from `scope`.
    pub fn var_port(&self, scope: ScopeId, d: yavs_sem::DeclId) -> Option<VarObj> {
        let mut cur = Some(scope);
        while let Some(s) = cur {
            if let Some(&v) = self.var_ports.get(&(s, d)) {
                return Some(v);
            }
            cur = self.hier.scope(s).parent;
        }
        None
    }

    /// The signal object declaration `d` denotes, seen from `scope`.
    pub fn signal(&self, scope: ScopeId, d: yavs_sem::DeclId) -> Option<ObjId> {
        let mut cur = Some(scope);
        while let Some(s) = cur {
            if let Some(&o) = self.sigs.get(&(s, d)) {
                return Some(o);
            }
            cur = self.hier.scope(s).parent;
        }
        // A package signal is visible from every region (#183).
        self.sigs
            .iter()
            .find(|((s, x), _)| {
                *x == d && matches!(self.hier.scope(*s).kind, ScopeKind::Package { .. })
            })
            .map(|(_, &o)| o)
    }

    /// The value of generic or constant `d` seen from `scope`, if elaboration computed it.
    pub fn value(&self, scope: ScopeId, d: yavs_sem::DeclId) -> Option<&Val> {
        let mut cur = Some(scope);
        while let Some(s) = cur {
            if let Some(v) = self.model_vals.get(s.index()).and_then(|m| m.get(&d)) {
                return Some(v);
            }
            cur = self.hier.scope(s).parent;
        }
        None
    }
}

/// Find the top level `name` (`ENTITY`, `ENTITY.ARCH` or `CONFIGURATION`) in library `work`.
pub fn find_top(db: &mut Db, work: &str, name: &str) -> Result<Top, Vec<Diagnostic>> {
    walk::find_top(db, work, name)
}

/// Elaborate `top` with `-g` overrides `(name, value text)`.
pub fn elaborate(
    db: &mut Db,
    top: Top,
    overrides: &[(String, String)],
    cfg: &ElabConfig,
) -> Result<Elaborated, Vec<Diagnostic>> {
    walk::elaborate(db, top, overrides, cfg)
}
