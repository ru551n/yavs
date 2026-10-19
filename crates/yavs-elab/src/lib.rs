//! Elaboration (docs/design/ir.md section 4): an analyzed design (yavs-sem HIR) becomes a
//! hierarchy of scopes, a signal net graph and a list of process instances, which
//! [`Elaborated::build_with`] turns into a kernel model.
//!
//! [`elaborate`] is a pure function of (design, top, generics, configuration): it keeps no
//! global state, so it can run again in the same process (rule 4, GHDL #1184), and building
//! the kernel model from one [`Elaborated`] can be repeated for an in-process restart.
//!
//! Fail early: any error-severity diagnostic means there is no [`Elaborated`].

mod build;
mod eval;
mod hier;
mod pkgcall;
mod sim;
mod value;
mod walk;
mod whole;

#[cfg(test)]
mod tests;

pub use eval::{Class, ConvFn, Enc, Model, Side, Val, elem_of};
pub use hier::{
    Bounds, Case, DriverDecl, Hierarchy, ImplicitKind, Leaf, LeafId, LeafRes, NetGraph, ObjId,
    ObjKind, PortEdge, ProcIdx, Scope, ScopeId, ScopeKind, Sel, Shape, SigObj,
};
pub use sim::Ir;

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
    /// Not implemented yet in yavs (names the slice or milestone).
    pub const NOT_IMPLEMENTED: &str = "E9999";
    /// Internal error: a broken invariant ("this is a bug in yavs").
    pub const INTERNAL: &str = "E9999";
}

/// Elaboration settings.
#[derive(Clone, Debug)]
pub struct ElabConfig {
    /// Instances nested deeper than this are an error (recursive designs end at a generate).
    pub max_depth: u32,
}

impl Default for ElabConfig {
    fn default() -> Self {
        ElabConfig { max_depth: 1000 }
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
    pub actual: Option<PortActual>,
}

/// The equivalent process `formal <= expr` of a port's expression actual: its one driver is
/// the formal, or the formal's part of shape `shape`; `formal` is the formal's subtype when
/// the whole formal is associated.
pub struct PortActual {
    pub expr: yavs_sem::ExprId,
    pub formal: Option<yavs_sem::SubtypeId>,
    pub shape: Shape,
}

pub struct Elaborated {
    pub top: Top,
    pub hier: Hierarchy,
    pub nets: NetGraph,
    pub procs: Vec<ProcInst>,
    /// Values of generics, generate parameters and constants per scope (for lowering).
    pub model_vals: Vec<std::collections::HashMap<yavs_sem::DeclId, Val>>,
    /// Signal objects by (scope, declaration).
    pub sigs: std::collections::HashMap<(ScopeId, yavs_sem::DeclId), ObjId>,
    /// The protected object of each 2019 variable port, by (scope of the formal, formal).
    pub var_ports: std::collections::BTreeMap<(ScopeId, yavs_sem::DeclId), VarObj>,
    /// Warnings of a successful elaboration.
    pub warnings: Vec<Diagnostic>,
    /// Conversion functions of port associations (`PortEdge::to_formal`/`to_actual`).
    pub convs: Vec<eval::ConvFn>,
    /// The IR of the processes, once [`Elaborated::lower`] ran.
    pub ir: Option<Ir>,
    /// Calls that ran with package state during elaboration, in order: replayed when the
    /// simulation is built (#77).
    pub(crate) elab_calls: Vec<pkgcall::Call>,
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
