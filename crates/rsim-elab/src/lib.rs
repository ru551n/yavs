//! Elaboration (docs/design/ir.md section 4): an analyzed design (rsim-sem HIR) becomes a
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
mod walk;

#[cfg(test)]
mod tests;

pub use eval::{Model, Val, elem_of};
pub use hier::{
    Bounds, Case, DriverDecl, Hierarchy, Leaf, LeafId, LeafRes, NetGraph, ObjId, ObjKind, PortEdge,
    ProcIdx, Scope, ScopeId, ScopeKind, Sel, Shape, SigObj,
};

use rsim_diag::Diagnostic;
use rsim_sem::{Db, StmtId, UnitId};

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
    /// Not implemented yet in rsim (names the slice or milestone).
    pub const NOT_IMPLEMENTED: &str = "E9999";
    /// Internal error: a broken invariant ("this is a bug in rsim").
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

/// What `rsim elab TOP` names: an entity, `entity.arch` or a configuration.
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
    pub lsps: Vec<Vec<(usize, LeafId, rsim_kernel::Range)>>,
}

pub struct Elaborated {
    pub top: Top,
    pub hier: Hierarchy,
    pub nets: NetGraph,
    pub procs: Vec<ProcInst>,
    /// Values of generics, generate parameters and constants per scope (for lowering).
    pub model_vals: Vec<std::collections::HashMap<rsim_sem::DeclId, Val>>,
    /// Signal objects by (scope, declaration).
    pub sigs: std::collections::HashMap<(ScopeId, rsim_sem::DeclId), ObjId>,
    /// Warnings of a successful elaboration.
    pub warnings: Vec<Diagnostic>,
}

impl Elaborated {
    /// The signal object declaration `d` denotes, seen from `scope`.
    pub fn signal(&self, scope: ScopeId, d: rsim_sem::DeclId) -> Option<ObjId> {
        let mut cur = Some(scope);
        while let Some(s) = cur {
            if let Some(&o) = self.sigs.get(&(s, d)) {
                return Some(o);
            }
            cur = self.hier.scope(s).parent;
        }
        None
    }

    /// The value of generic or constant `d` seen from `scope`, if elaboration computed it.
    pub fn value(&self, scope: ScopeId, d: rsim_sem::DeclId) -> Option<&Val> {
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
