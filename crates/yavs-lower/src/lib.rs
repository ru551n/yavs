//! Lowering of yavs-sem's typed HIR into yavs-ir (`DESIGN.md` in this crate).
//!
//! [`Lowerer`] turns analyzed design units into one [`yavs_ir::Program`]: one `Func` per
//! process and subprogram body, shared by every instance of its design unit (ir.md 2), and a
//! [`UnitIr`] per architecture describing its instance data, signals and processes for the
//! elaborator. Subprograms are lowered on first reference, so the std and ieee bodies a design
//! never calls are not lowered. Every instruction carries the statement it belongs to.
//!
//! Fail early: a construct this crate does not lower yet is an `E2001` diagnostic naming it,
//! never a guessed value; a broken HIR invariant is `E9999`. Any error means no program.

mod access;
mod assoc;
mod composite;
mod covcond;
mod edge;
mod expr;
mod file;
mod force;
mod foreign;
mod gather;
pub use foreign::vhpi_model;
pub use pathname::{library_package_item_name, package_prefix, subprogram_element};
mod func;
mod pathname;
mod protected;
mod reduce;
mod reflect;
mod stmt;
mod ty;

use std::collections::HashMap;

use yavs_diag::{Diagnostic, Span};
use yavs_ir::{
    CovInfo, CovItem, CovKind, FuncId, FuncKind, NativeId, Program, ProgramBuilder, TyId,
    VerifyError, Width,
};
use yavs_sem::decl::DeclKind;
use yavs_sem::hir::UnitHir;
use yavs_sem::{Db, DeclId, ExprId, ScopeId, StmtId, SubtypeId, TypeId, UnitId};

pub use expr::deferred_completion;
pub use ty::{encode_array, narrow_elem, signal_elem};

/// A construct the lowering does not handle yet (yavs-diag registry: `E2000`-`E2099`).
pub const NOT_IMPLEMENTED: &str = "E2001";
/// Internal compiler error.
pub const INTERNAL: &str = "E9999";

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct LowerOptions {
    /// Emit statement and branch coverage counters (`Cover`, rule 10).
    pub coverage: bool,
    /// Emit condition coverage (`Inst::CoverIn`, `Inst::CoverCond`).
    pub cov_condition: bool,
    /// Emit expression coverage of the values of assignments.
    pub cov_expression: bool,
    /// Run the subprograms of `yavs_ir::ieee` from their VHDL bodies (the environment variable `YAVS_NO_NATIVE_IEEE`).
    pub no_native_ieee: bool,
}

impl LowerOptions {
    /// Any counters at all: code lowered for a run without them is not the code to reuse.
    pub fn any_coverage(&self) -> bool {
        self.coverage || self.cov_condition || self.cov_expression
    }
}

impl Default for LowerOptions {
    /// The environment variable `YAVS_NO_NATIVE_IEEE` (any value) turns the native `ieee`
    /// subprograms off everywhere, elaboration-time evaluation included.
    fn default() -> Self {
        Self {
            coverage: false,
            cov_condition: false,
            cov_expression: false,
            no_native_ieee: std::env::var_os("YAVS_NO_NATIVE_IEEE").is_some(),
        }
    }
}

/// The lowered form of an architecture: what the elaborator needs to build its instances.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct UnitIr {
    pub unit: UnitId,
    /// Bytes of instance data (`AddrSpace::Inst`).
    pub inst_size: u32,
    /// Generics and constants whose values the elaborator supplies (`Elaborated::value`),
    /// written into instance data with [`UnitIr::write_value`], in declaration order.
    pub values: Vec<InstValue>,
    /// Scalar constants the elaborator could not fold: it runs each thunk (it may read the
    /// values before it) and stores the returned cell at `off` with width `w`.
    pub consts: Vec<InstConst>,
    /// Signals, indexed by `Inst::SigVal::sig`.
    pub signals: Vec<SignalIr>,
    /// Processes in elaboration order, concurrent statements included.
    pub processes: Vec<ProcessIr>,
    /// `'path_name` and `'instance_name` strings the elaborator writes with
    /// [`UnitIr::write_string`] (LRM 16.2.5, lower case).
    pub names: Vec<InstName>,
    /// 2019 variable ports (LRM 6.5.2): the 8-byte cell at the offset holds the handle of the
    /// protected object the port's actual denotes, written by the elaborator (#188).
    pub var_ports: Vec<(DeclId, u32)>,
    /// Aliases of external shared variables (#93): the elaborator stores the variable's
    /// object (the cell its own region holds) at `off`.
    pub ext_vars: Vec<InstValue>,
    /// The ranges of types and subtypes with a bound that is not globally static: the
    /// elaborator evaluates each once, at its declaration, and writes it here (#612).
    pub ranges: Vec<InstRange>,
}

/// A range of a declared type or subtype (`Db::dynamic_range`): its left and right bounds as
/// two i64 at `off`, as the elaborator evaluated them in the region `scope` (the entity,
/// architecture, block or generate declaring it).
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct InstRange {
    pub left: yavs_sem::ExprId,
    pub off: u32,
    pub scope: Option<ScopeId>,
}

/// A `'path_name` (`instance: false`) or `'instance_name` of named entity `decl`, as seen from
/// the region whose instance data is being built; an array slot at `off`.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct InstName {
    pub decl: DeclId,
    pub instance: bool,
    pub off: u32,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct InstConst {
    pub decl: DeclId,
    pub name: String,
    pub off: u32,
    pub w: Width,
    /// A composite constant's IR type: the thunk returns the address of its fat pair
    /// `[data, desc]`, written like a composite [`InstValue`] (`w` unused).
    pub comp: Option<TyId>,
    /// `FuncKind::Thunk` returning the value.
    pub init: FuncId,
    /// Declared in the entity or architecture itself (not in a block or generate): one value
    /// per instance, shared by the instance data of all its regions. A shared variable's
    /// thunk creates its object, so it must run once per instance (LRM 6.4.2.4); one of a
    /// block or generate runs once per declaring region (yavs-elab, #249).
    pub instance_level: bool,
    /// The entity, architecture, block or generate whose elaboration creates the constant: its
    /// thunk runs only for the elaborated regions inside it (a generate that was not
    /// elaborated has none, #495); `None` runs it everywhere.
    pub scope: Option<ScopeId>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct SignalIr {
    pub decl: DeclId,
    /// Original spelling (rule 8).
    pub name: String,
    pub ty: TyId,
    /// `FuncKind::Thunk` returning the initial value of a scalar signal (`None` for ports
    /// and arrays: the elaborator computes initial values itself).
    pub init: Option<FuncId>,
    /// The leaf of a record signal: field indices from the signal's root (empty for a
    /// signal that is one kernel signal). A record signal has one entry per leaf, in leaf
    /// order (docs/decisions.md "Elaboration": one leaf per field).
    pub path: Vec<u32>,
    /// The descriptor of the signal's whole type (`Ty::desc_size` of the second element
    /// bytes at the offset in instance data), written by the elaborator from the signal's
    /// bounds in each instance: ports take their actual's bounds. Array signals have one, and
    /// signals with records whose type needs a descriptor have it on their first leaf entry.
    /// In `PackageIr::signals` the offset is in package data (written once after the
    /// package's initialisation); the package entries of `UnitIr::signals` have none.
    pub dope: Option<(u32, TyId)>,
}

/// A generic or constant in instance data.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct InstValue {
    pub decl: DeclId,
    pub name: String,
    pub off: u32,
    /// Its IR type: a scalar is one cell of the type's width at `off`; an array has the byte
    /// offsets of its elements and of its descriptor (both in instance data) at `off` and
    /// `off + 8`.
    pub ty: TyId,
}

impl UnitIr {
    /// Write string `s` (a STRING `1 to len`) into the array slot at `off` of `data`.
    pub fn write_string(off: u32, s: &str, data: &mut Vec<u8>) -> Result<(), String> {
        let off = off as usize;
        if data.len() < off + 16 {
            return Err("instance data too short for a name".into());
        }
        let mut put = |bytes: &[u8]| {
            let at = data.len().next_multiple_of(8);
            data.resize(at, 0);
            data.extend_from_slice(bytes);
            at as u64
        };
        let e = put(s.as_bytes());
        let dope: Vec<u8> = [1i64, s.len() as i64, 0]
            .iter()
            .flat_map(|x| x.to_le_bytes())
            .collect();
        let d = put(&dope);
        data[off..off + 8].copy_from_slice(&e.to_le_bytes());
        data[off + 8..off + 16].copy_from_slice(&d.to_le_bytes());
        Ok(())
    }

    /// Write value `v` of `iv` into instance data `data` (at least `inst_size` bytes): a
    /// scalar into its cell, an array appended (elements in the IR layout, bit-planes for
    /// logic arrays, then the descriptor) with the offsets stored at `iv.off`.
    pub fn write_value(
        prog: &Program,
        iv: &InstValue,
        v: &yavs_sem::stat::ConstValue,
        data: &mut Vec<u8>,
    ) -> Result<(), String> {
        let off = iv.off as usize;
        let t = prog.types.get(iv.ty).ok_or("missing type")?;
        if data.len() < off + t.scalar_size().map_or(16, |n| n as usize) {
            return Err(format!("instance data too short for `{}`", iv.name));
        }
        if let Some(n) = t.scalar_size() {
            let c = ty::cell(v).ok_or_else(|| format!("`{}`: not a scalar value", iv.name))?;
            data[off..off + n as usize].copy_from_slice(&c.to_le_bytes()[..n as usize]);
            return Ok(());
        }
        let (elems, dope) = ty::encode_array(prog, iv.ty, v)
            .ok_or_else(|| format!("`{}`: value does not fit its array type", iv.name))?;
        Self::write_composite(iv.off, &elems, &dope, data);
        Ok(())
    }

    /// Append a composite's elements and descriptor to `data` and write their offsets into
    /// the slot at `off` (an `InstValue` of composite type, which `data` must cover).
    pub fn write_composite(off: u32, elems: &[u8], desc: &[u8], data: &mut Vec<u8>) {
        let off = off as usize;
        let mut put = |bytes: &[u8]| {
            let at = data.len().next_multiple_of(8);
            data.resize(at, 0);
            data.extend_from_slice(bytes);
            at as u64
        };
        let e = put(elems);
        let d = put(desc);
        data[off..off + 8].copy_from_slice(&e.to_le_bytes());
        data[off + 8..off + 16].copy_from_slice(&d.to_le_bytes());
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct ProcessIr {
    /// The HIR statement (a process or a concurrent statement).
    pub stmt: StmtId,
    pub func: FuncId,
    pub label: Option<String>,
    pub postponed: bool,
    /// Driver slots, one per `ProcessInfo::drivers` entry in order: the instance signal it
    /// drives (the elaborator gives each slot the entry's kernel range).
    pub drivers: Vec<u32>,
    /// The equivalent process of a port's expression actual, `stmt` being the instance or
    /// block statement ([`Lowerer::port_actual`]).
    pub actual: Option<PortActual>,
}

/// The equivalent process `formal <= expr` of a port associated with a non-static expression
/// (LRM 6.5.6.3), sensitive to the expression's signals. It has no driver slots of its own:
/// the elaborator gives it one, slot 0, for the formal (or the formal's sub-element).
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct PortActual {
    pub expr: ExprId,
    /// The IR type of the formal (the expression's type).
    pub ty: TyId,
    /// For an array: instance data where the elaborator writes the formal's descriptor
    /// (`Ty::desc_size` bytes), against which the value is checked.
    pub dope: Option<u32>,
    /// For a scalar formal whose range is not static: instance data where the elaborator
    /// writes the range as three 8-byte words, left and right (f64 bits for a real) and the
    /// direction (0 for `to`), against which the value is checked.
    pub bounds: Option<u32>,
}

#[derive(Debug)]
pub struct Lowered {
    pub program: Program,
    pub units: Vec<UnitIr>,
    /// Packages with run-time objects, in the order [`Lowerer::package`] was called; the
    /// `index` of each is its `AddrSpace::Pkg` number.
    pub packages: Vec<PackageIr>,
    /// Bytes of the shared variables area (`SharedState::vars`): file objects and
    /// non-protected shared variables of packages.
    pub shared_size: u32,
}

/// A non-protected shared variable, its slot thunk (packages) and its value's IR type
/// (`Lowerer::var_slots`).
pub type VarSlot = (DeclId, Option<FuncId>, TyId);

/// A package's objects at run time (LRM 14.4.2). The elaborator initialises each package
/// once, in dependency order, with `yavs_interp::init_package(rt, init, index, size, shared)`
/// and binds the results as `Bindings::packages` (index order); signals are the first
/// entries of every `UnitIr::signals` (the same index in every instance).
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct PackageIr {
    /// The package declaration (its body, if analyzed, is included).
    pub unit: UnitId,
    pub index: u32,
    /// Bytes of package data.
    pub size: u32,
    /// `FuncKind::Thunk`: evaluates the constants and creates the shared variables.
    pub init: FuncId,
    pub signals: Vec<SignalIr>,
    /// Aliases of external constants and shared variables used in the package's subprograms
    /// (#255): cells in the shared variables area (`off`) where the elaborator writes the
    /// constant's value (as [`UnitIr::write_value`] does, offsets relative to the area) or the
    /// variable's object once the design is elaborated. Their signals are package signals.
    pub ext_consts: Vec<InstValue>,
    pub ext_vars: Vec<InstValue>,
}

/// Lowering failed; the diagnostics say why.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Failed;

pub(crate) type R<T> = Result<T, Failed>;

/// Where an object lives.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug)]
pub(crate) enum Obj {
    /// A scalar of width `w` at `off` in `space`.
    Scalar { space: Space, off: u32, w: Width },
    /// A scalar `out`/`inout` parameter: the slot holds the address of the actual.
    ScalarRef { space: Space, off: u32, w: Width },
    /// An array: a fat pair `[data, dope]` at `off`.
    Array { space: Space, off: u32 },
    /// Instance signal `sig`.
    Signal { sig: u32 },
    /// A record signal: one instance signal per leaf (`Cx::rec_leaves`), or a signal
    /// parameter with records in its type (`RecParam`).
    RecSignal(Option<RecParam>),
    /// A composite of static layout stored in place: `size` bytes of elements at `off`, then
    /// its descriptor.
    Fixed { space: Space, off: u32, size: u32 },
    /// An array in instance data (or the shared variables area): byte offsets of its
    /// elements and descriptor at `off`.
    InstArray { space: Space, off: u32 },
    /// A signal parameter: the slot holds a signal reference (two cells).
    /// With `out`: an `out`/`inout` signal parameter, the caller's driver slot in the cell
    /// at `off + 16`.
    SignalRef { space: Space, off: u32, out: bool },
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Space {
    /// The frame of the function at this nesting depth.
    Frame(u8),
    Inst,
    /// Package data (`AddrSpace::Pkg`).
    Pkg(u32),
    /// The shared variables area (`AddrSpace::Shared`, serial processes only).
    Shared,
    /// The shared heap block whose address is in the instance cell at this offset: a
    /// non-protected shared variable of an architecture, block or generate, one per declaring
    /// region (serial processes only, like `Shared`).
    Region(u32),
}

/// The slot of a variable that is not of a protected or access type ([`func::Fx::var_init`]).
#[derive(Clone, Copy, Debug)]
pub(crate) enum VarShape {
    Scalar(Width),
    /// An array of scalars with this many dimensions: `[data, dope, handle, descriptor]`.
    Array(u8),
    /// A record or an array of composites: `[data, desc, handle]`.
    Comp,
}

impl VarShape {
    pub fn size(self) -> u32 {
        match self {
            VarShape::Scalar(w) => w.bytes(),
            VarShape::Array(dims) => 24 + 24 * u32::from(dims),
            VarShape::Comp => 24,
        }
    }

    /// The object of a slot of this shape at `off` in `space`.
    pub fn obj(self, space: Space, off: u32) -> Obj {
        match self {
            VarShape::Scalar(w) => Obj::Scalar { space, off, w },
            VarShape::Array(_) | VarShape::Comp => Obj::Array { space, off },
        }
    }
}

/// State shared by the functions being lowered.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct Cx {
    pub opts: LowerOptions,
    /// Files of the std/ieee libraries yavs provides: no coverage counters there.
    pub cov_skip: std::collections::HashSet<yavs_diag::FileId>,
    /// Empty in an image (`Lowerer::image` refuses otherwise).
    #[serde(skip)]
    pub diags: Vec<Diagnostic>,
    pub tys: HashMap<TypeId, TyId>,
    pub natives: HashMap<(String, Option<TyId>), NativeId>,
    pub foreigns: HashMap<DeclId, yavs_ir::ForeignId>,
    pub funcs: HashMap<DeclId, FuncId>,
    /// Subprograms declared but not lowered yet, with their nesting depth.
    pub queue: Vec<(DeclId, u8)>,
    /// Function and depth of each subprogram or process region.
    pub scope_func: HashMap<ScopeId, (FuncId, u8)>,
    /// Driver LSPs and slot bases of each process region: subprograms declared in a process
    /// drive through its drivers (LRM 14.7.2).
    pub proc_drivers: HashMap<ScopeId, (Vec<yavs_sem::hir::Lsp>, Vec<u32>)>,
    /// Protected type, frame depth and `self` slot of each method body region: subprograms
    /// declared in a method act on its object (LRM 5.6.2, #221).
    pub method_self: HashMap<ScopeId, (TypeId, u8, u32)>,
    pub objs: HashMap<DeclId, Obj>,
    /// The frame slot of each non-static range of a type or subtype declared in a sequential
    /// region, by its left bound: evaluated once at the declaration (LRM 14.4.2.1, #612).
    pub decl_ranges: HashMap<yavs_sem::ExprId, (Space, u32)>,
    /// Array variables (`Obj::Array`) whose descriptor is a constant: reads use it instead of
    /// loading the descriptor pointer from the slot (the optimiser then folds bounds).
    pub const_dope: HashMap<DeclId, yavs_ir::ConstId>,
    /// The leaf table of a record signal read and driven as a whole (`gather.rs`), `None` when
    /// the record has none.
    pub rec_tables: HashMap<(DeclId, Vec<u32>, yavs_ir::TyId), Option<yavs_ir::ConstId>>,
    /// `std.reflection` entries per subtype (`reflect.rs`).
    pub refl: HashMap<yavs_sem::SubtypeId, yavs_ir::ReflId>,
    /// Instance data of the architecture being lowered: bytes allocated so far, and the
    /// `'path_name`/`'instance_name` slots its code reads.
    pub inst_size: u32,
    pub names: Vec<InstName>,
    /// Descriptor slots of array signals (`SignalIr::dope`): instance data, or package data
    /// for package signals.
    pub sig_dopes: HashMap<DeclId, (Space, u32)>,
    /// Signals of packages: the first entries of every unit's signal table.
    pub pkg_sigs: Vec<SignalIr>,
    /// The entity and architecture regions of the architecture being lowered.
    pub arch_scopes: Vec<ScopeId>,
    /// Bytes of the shared variables area allocated so far (package files and non-protected
    /// shared variables).
    pub shared_size: u32,
    /// Non-protected shared variables lowered so far and the IR types of their values
    /// (`Lowerer::var_slots`).
    pub shared_vars: Vec<(DeclId, TyId)>,
    /// The leaves of record signals, in leaf order.
    pub rec_leaves: HashMap<DeclId, Vec<RecLeaf>>,
    /// Port actuals that get an equivalent process ([`Lowerer::port_actual`]): the instance
    /// statement, the formal's subtype for the scalar range check and the port.
    pub actuals: HashMap<ExprId, (StmtId, Option<SubtypeId>, DeclId)>,
    /// External signal names aliased in the unit being lowered: the alias that holds the
    /// signal (`func::external_alias`).
    pub ext_sigs: HashMap<yavs_sem::ExternalId, DeclId>,
    /// External aliases of subprograms (`ExtOwner::Sub`) that are not bound, with why: a
    /// subprogram using one fails when it is called. `None` before the design is elaborated
    /// (elaboration-time calls): none is bound yet (#255).
    pub ext_unbound: Option<HashMap<DeclId, String>>,
    /// A subprogram using external names was lowered before `ext_unbound` was known: it fails
    /// on entry ([`Lowerer::externals_early`]).
    pub ext_early: bool,
    /// The functions whose code depends on the design being elaborated: they allocated instance
    /// data of the architecture being lowered for a `'path_name` or `'instance_name`, or fail
    /// on entry for an external name that is not bound yet, or failed to lower
    /// (`Lowerer::context_dependent`).
    #[serde(skip)]
    pub context_funcs: Vec<FuncId>,
    /// Elaboration-time calls: constants and generics of the enclosing regions are read from
    /// slots of instance data the elaborator fills ([`Lowerer::capture_free_objects`]).
    #[serde(skip)]
    pub capture_free: bool,
    /// Also the protected shared variables of those regions: each a cell holding the handle of
    /// its object ([`Lowerer::capture_free_objects`]).
    #[serde(skip)]
    pub capture_vars: bool,
    #[serde(skip)]
    pub free_objs: Vec<FreeObj>,
    /// Which function reads which of `free_objs` (index), directly.
    #[serde(skip)]
    pub free_readers: Vec<(usize, FuncId)>,
    /// The thunks that create the object of a captured shared variable ([`Lowerer::region_var_init`]).
    #[serde(skip)]
    pub region_inits: HashMap<DeclId, FuncId>,
}

/// A constant or generic of an enclosing region that a function called during elaboration
/// reads: the elaborator writes its value to the instance data slot at `off`. A shared variable
/// (`var`) has the handle of its object there.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct FreeObj {
    pub decl: DeclId,
    pub ty: TyId,
    pub off: u32,
    pub var: bool,
}

/// One kernel signal of a record signal.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub(crate) struct RecLeaf {
    pub path: Vec<u32>,
    pub st: yavs_sem::SubtypeId,
    /// The instance signal; for a signal parameter, the leaf's index (`RecParam`).
    pub sig: u32,
}

/// A signal parameter with records in its type (DESIGN.md "Contract with yavs-interp and
/// yavs-elab"): from `off` in `space`, one signal reference (two cells) per leaf of the
/// formal, in leaf order; then, with `out`, the caller's driver slot of the first leaf (leaf
/// `k` is that slot plus `k`); then, with `desc`, the address of the actual's descriptor.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RecParam {
    pub space: Space,
    pub off: u32,
    pub leaves: u32,
    pub out: bool,
    pub desc: bool,
}

impl RecParam {
    /// The offset of the driver slot cell (`out`).
    pub fn slot_off(&self) -> u32 {
        self.off + 16 * self.leaves
    }

    /// The offset of the descriptor address cell (`desc`).
    pub fn desc_off(&self) -> u32 {
        self.slot_off() + if self.out { 8 } else { 0 }
    }
}

impl Cx {
    /// `size` bytes of instance data of the architecture being lowered (8-byte aligned).
    pub fn inst_alloc(&mut self, size: u32) -> u32 {
        let off = self.inst_size.next_multiple_of(8);
        self.inst_size = off + size;
        off
    }

    /// The declaration a signal LSP root names: an object, or the alias of an external
    /// signal name, or the package signal a package path names.
    pub fn root_decl(&self, db: &Db, r: yavs_sem::hir::ObjRoot) -> Option<DeclId> {
        match r {
            yavs_sem::hir::ObjRoot::Decl(d) => Some(d),
            yavs_sem::hir::ObjRoot::External(x) => {
                self.ext_sigs.get(&x).copied().or(db.package_target(x))
            }
            yavs_sem::hir::ObjRoot::ImplicitSignal(_) => None,
        }
    }

    pub fn error(&mut self, code: &'static str, span: Span, msg: impl Into<String>) -> Failed {
        self.diags
            .push(Diagnostic::error(code, msg.into()).with_primary(span, ""));
        Failed
    }

    /// The diagnostics of a failed lowering. A `Failed` that reached here without any
    /// diagnostic is a bug: it becomes an ICE instead of a silent exit (error motto).
    pub fn take_failed(&mut self) -> Vec<Diagnostic> {
        if self.diags.is_empty() {
            self.diags.push(Diagnostic::error(
                INTERNAL,
                "internal error: lowering failed without a diagnostic; this is a bug in yavs",
            ));
        }
        std::mem::take(&mut self.diags)
    }

    pub fn unsupported(&mut self, span: Span, what: &str) -> Failed {
        self.error(
            NOT_IMPLEMENTED,
            span,
            format!("{what} is not lowered to IR yet (M2)"),
        )
    }

    pub fn ice(&mut self, span: Span, what: &str) -> Failed {
        self.error(
            INTERNAL,
            span,
            format!("internal error: {what}; this is a bug in yavs"),
        )
    }

    /// Whether a condition (`expr`: an expression) in file `file` is measured.
    pub fn cov_cond_on(&self, expr: bool, file: yavs_diag::FileId) -> bool {
        let on = if expr {
            self.opts.cov_expression
        } else {
            self.opts.cov_condition
        };
        on && !self.cov_skip.contains(&file)
    }

    /// The counter of row `kind` of a condition or expression at `span`.
    pub fn cover_cond(
        &mut self,
        pb: &mut ProgramBuilder,
        stmt: yavs_ir::StmtId,
        span: Span,
        kind: CovKind,
        row: yavs_ir::CovRow,
    ) -> CovItem {
        let _ = span;
        pb.cov(CovInfo {
            stmt,
            kind,
            row: Some(row),
        })
    }

    pub fn cover(
        &mut self,
        pb: &mut ProgramBuilder,
        stmt: yavs_ir::StmtId,
        span: Span,
        kind: CovKind,
    ) -> Option<CovItem> {
        (self.opts.coverage && !self.cov_skip.contains(&span.file)).then(|| {
            pb.cov(CovInfo {
                stmt,
                kind,
                row: None,
            })
        })
    }
}

/// The constant bytes up to which [`Lowerer::image`] makes an image.
pub const IMAGE_MAX_CONSTS: usize = 1 << 20;

/// [`Lowerer::image`].
#[derive(serde::Serialize)]
struct ImageRef<'a> {
    opts: LowerOptions,
    pb: &'a ProgramBuilder,
    cx: &'a Cx,
    units: &'a [UnitIr],
    packages: &'a [PackageIr],
}

#[derive(serde::Deserialize)]
struct Image {
    opts: LowerOptions,
    pb: ProgramBuilder,
    cx: Cx,
    units: Vec<UnitIr>,
    packages: Vec<PackageIr>,
}

/// A [`Lowerer`] set aside without its `Db` ([`Lowerer::suspend`]).
pub struct Suspended {
    pb: ProgramBuilder,
    cx: Cx,
    units: Vec<UnitIr>,
    packages: Vec<PackageIr>,
}

impl Suspended {
    /// [`Lowerer::externals_early`] of the lowering set aside.
    pub fn externals_early(&self) -> bool {
        self.cx.ext_early
    }

    /// [`Lowerer::restore`] before [`Lowerer::resume`].
    pub fn restore(&mut self, program: Program) {
        self.pb.restore(program);
    }

    /// [`Lowerer::forget_free_objects`] for a lowering set aside, after [`Suspended::restore`].
    pub fn forget_free_objects(&mut self) {
        forget_free_objects(&mut self.pb, &mut self.cx);
    }
}

pub struct Lowerer<'a> {
    db: &'a Db,
    pb: ProgramBuilder,
    cx: Cx,
    units: Vec<UnitIr>,
    packages: Vec<PackageIr>,
}

impl<'a> Lowerer<'a> {
    /// `db` must hold no error diagnostics (fail early: nothing after a failed stage).
    pub fn new(db: &'a Db, opts: LowerOptions) -> Self {
        Lowerer {
            db,
            pb: ProgramBuilder::new(),
            cx: Cx {
                opts,
                cov_skip: db
                    .units
                    .iter()
                    .filter(|(_, u)| {
                        matches!(
                            db.libs[u.lib].source,
                            yavs_sem::loader::LibSource::StdDir(_)
                        )
                    })
                    .map(|(_, u)| u.file)
                    .collect(),
                diags: Vec::new(),
                tys: HashMap::new(),
                natives: HashMap::new(),
                foreigns: HashMap::new(),
                funcs: HashMap::new(),
                queue: Vec::new(),
                scope_func: HashMap::new(),
                proc_drivers: HashMap::new(),
                method_self: HashMap::new(),
                objs: HashMap::new(),
                const_dope: HashMap::new(),
                rec_tables: HashMap::new(),
                refl: HashMap::new(),
                inst_size: 0,
                names: Vec::new(),
                sig_dopes: HashMap::new(),
                decl_ranges: HashMap::new(),
                pkg_sigs: Vec::new(),
                arch_scopes: Vec::new(),
                shared_size: 0,
                shared_vars: Vec::new(),
                rec_leaves: HashMap::new(),
                actuals: HashMap::new(),
                ext_sigs: HashMap::new(),
                ext_unbound: None,
                ext_early: false,
                context_funcs: Vec::new(),
                capture_free: false,
                capture_vars: false,
                free_objs: Vec::new(),
                free_readers: Vec::new(),
                region_inits: HashMap::new(),
            },
            packages: Vec::new(),
            units: Vec::new(),
        }
    }

    /// Let the functions lowered next read constants and generics declared in the regions of
    /// an instance (the elaboration-time call of a function declared in a block, say): each
    /// gets a slot of instance data, listed by [`Lowerer::free_objects`] for the caller to fill.
    /// With `vars`, the protected shared variables of those regions are captured too: a slot
    /// holding the handle of the object, which [`Lowerer::region_var_init`] creates. Functions
    /// lowered this way read instance data laid out for the call, not for an architecture, so
    /// they must be given up with [`Lowerer::forget_free_objects`] before the design is
    /// lowered.
    pub fn capture_free_objects(&mut self, vars: bool) {
        self.cx.capture_free = true;
        self.cx.capture_vars = vars;
    }

    /// The captured objects and the bytes of instance data they need.
    pub fn free_objects(&self) -> (Vec<FreeObj>, u32) {
        (self.cx.free_objs.clone(), self.cx.inst_size)
    }

    /// The captured objects that `f` reads, itself or through the functions it calls.
    pub fn free_for(&self, f: FuncId) -> Vec<FreeObj> {
        let mut out = Vec::new();
        for (i, o) in self.cx.free_objs.iter().enumerate() {
            let readers: Vec<FuncId> = self
                .cx
                .free_readers
                .iter()
                .filter(|&&(k, _)| k == i)
                .map(|&(_, r)| r)
                .collect();
            if readers.contains(&f) || self.pb.callers_of(&readers).contains(&f) {
                out.push(o.clone());
            }
        }
        out
    }

    /// A thunk that creates the object of shared variable `decl` (captured, or to be) and
    /// returns its handle, run with the instance data of the captured objects it reads.
    pub fn region_var_init(&mut self, decl: DeclId) -> Result<FuncId, Vec<Diagnostic>> {
        if let Some(&f) = self.cx.region_inits.get(&decl) {
            return Ok(f);
        }
        let r = func::region_var_init(self.db, &mut self.pb, &mut self.cx, decl);
        self.drain();
        match r {
            Ok(f) if self.cx.diags.is_empty() => {
                self.cx.region_inits.insert(decl, f);
                Ok(f)
            }
            _ => Err(self.cx.take_failed()),
        }
    }

    /// End the capture mode: the functions that read captured objects, and those that call
    /// them, are forgotten (lowered again when the design needs them, for its instance data),
    /// and so is every captured object. The old code stays in the program, unreferenced.
    pub fn forget_free_objects(&mut self) {
        forget_free_objects(&mut self.pb, &mut self.cx);
    }

    /// Lower an architecture: its signals, instance constants and processes.
    pub fn architecture(&mut self, unit: UnitId) -> Result<usize, Vec<Diagnostic>> {
        let r = func::architecture(self.db, &mut self.pb, &mut self.cx, unit);
        self.drain();
        match r {
            Ok(u) if self.cx.diags.is_empty() => {
                self.units.push(u);
                Ok(self.units.len() - 1)
            }
            _ => Err(self.cx.take_failed()),
        }
    }

    /// The design is elaborated: the external aliases of subprograms are bound except
    /// `unbound` (alias, why), which make their subprograms fail when called (#255). Call it
    /// before lowering any subprogram that uses external names.
    pub fn externals_bound(&mut self, unbound: HashMap<DeclId, String>) {
        self.cx.ext_unbound = Some(unbound);
    }

    /// The subprograms lowered so far that cannot be lowered ahead of the design: one that reads
    /// a `'path_name` or `'instance_name` (the name is a cell of the instance data of the
    /// architecture being lowered), one that uses an external name (it fails on entry until the
    /// name is bound), one that failed to lower, and every subprogram that calls such a one.
    pub fn context_dependent(&self) -> Vec<DeclId> {
        let mut seeds = self.cx.context_funcs.clone();
        seeds.extend(self.pb.undefined_funcs());
        if seeds.is_empty() {
            return Vec::new();
        }
        let mut tainted: std::collections::HashSet<FuncId> =
            self.pb.callers_of(&seeds).into_iter().collect();
        tainted.extend(seeds);
        let mut out: Vec<DeclId> = self
            .cx
            .funcs
            .iter()
            .filter(|(_, f)| tainted.contains(f))
            .map(|(d, _)| *d)
            .collect();
        out.sort_by_key(|d| yavs_sem::Id::index(*d));
        out
    }

    /// A digest of the ids lowering has handed out (`ProgramBuilder::fingerprint`).
    pub fn fingerprint(&self) -> u64 {
        self.pb.fingerprint()
    }

    /// How many functions depend on the design so far ([`Lowerer::context_dependent`]).
    pub fn context_users(&self) -> usize {
        self.cx.context_funcs.len()
    }

    /// Bytes of the shared variables area allocated so far.
    pub fn shared_size(&self) -> u32 {
        self.cx.shared_size
    }

    /// The number of functions in the program so far.
    pub fn funcs(&self) -> usize {
        self.pb.func_count()
    }

    /// The packages lowered so far ([`Lowerer::package`]).
    pub fn package_irs(&self) -> &[PackageIr] {
        &self.packages
    }

    /// Whether a subprogram using external names was lowered before
    /// [`Lowerer::externals_bound`]: it fails on entry, which is right during elaboration but
    /// not in the simulation.
    pub fn externals_early(&self) -> bool {
        self.cx.ext_early
    }

    /// Give the actual `expr` of port `port` in instance statement `stmt` (its port map or
    /// the port map of its binding indication) an equivalent process ([`PortActual`]), lowered
    /// with the architecture of `stmt` after this call; `formal` is the formal's subtype when
    /// the whole formal is associated (its range is checked).
    pub fn port_actual(
        &mut self,
        stmt: StmtId,
        expr: ExprId,
        formal: Option<SubtypeId>,
        port: DeclId,
    ) {
        self.cx.actuals.insert(expr, (stmt, formal, port));
    }

    /// Lower the run-time objects of package `unit` (and of its body, when analyzed): call it
    /// for every package with objects before lowering the architectures that use them.
    pub fn package(&mut self, unit: UnitId) -> Result<usize, Vec<Diagnostic>> {
        let index = self.packages.len() as u32;
        let r = func::package(self.db, &mut self.pb, &mut self.cx, unit, index);
        self.drain();
        match r {
            Ok(p) if self.cx.diags.is_empty() => {
                self.packages.push(p);
                Ok(self.packages.len() - 1)
            }
            _ => Err(self.cx.take_failed()),
        }
    }

    /// A thunk returning the value of constant `decl` of a package lowered with
    /// [`Lowerer::package`]: a scalar's cell, or the address of a composite's fat pair
    /// `[data, desc]`. For the elaborator, which reads package constants with impure
    /// initial values from the package data instead of evaluating them again (#77).
    pub fn package_value(&mut self, decl: DeclId) -> Result<FuncId, Vec<Diagnostic>> {
        let d = &self.db.decls[decl];
        let (name, span) = (format!("{}'value", d.name), d.span);
        let f = func::thunk(self.db, &mut self.pb, &mut self.cx, &name, span, |fx| {
            fx.obj_cell(decl, span)
        });
        self.drain();
        match f {
            Ok(f) if self.cx.diags.is_empty() => Ok(f),
            _ => Err(self.cx.take_failed()),
        }
    }

    /// A function `(k : integer) return integer` returning bound `k` (left, right, left, ...)
    /// of the ranges package type or subtype `decl` evaluated into package data
    /// ([`Lowerer::package`]). For the elaborator, which reads them instead of evaluating
    /// them again (#612).
    pub fn package_range(&mut self, decl: DeclId) -> Result<FuncId, Vec<Diagnostic>> {
        let d = &self.db.decls[decl];
        let (name, span) = (format!("{}'ranges", d.name), d.span);
        let first = (self.db.declared_ranges(decl).into_iter())
            .find_map(|r| self.db.dynamic_range(r))
            .and_then(|l| self.cx.decl_ranges.get(&l).copied());
        let f = func::thunk(self.db, &mut self.pb, &mut self.cx, &name, span, |fx| {
            let Some((space @ Space::Pkg(_), off)) = first else {
                return Err(fx.cx.ice(span, "package ranges not lowered"));
            };
            let int = yavs_ir::RegTy::I64;
            let k = (fx.fb).param(
                "k",
                Width::B8,
                yavs_ir::ParamClass::Value,
                yavs_ir::Mode::In,
                None,
            );
            let ka = fx.addr(Space::Frame(0), k);
            let pos = fx.load(ka, Width::B8, int);
            let base = fx.addr(space, off);
            let dst = fx.reg(yavs_ir::RegTy::Ptr);
            fx.emit(yavs_ir::Inst::Index {
                dst,
                base,
                pos,
                stride: 8,
            });
            Ok(fx.load(dst, Width::B8, int))
        });
        self.drain();
        match f {
            Ok(f) if self.cx.diags.is_empty() => Ok(f),
            _ => Err(self.cx.take_failed()),
        }
    }

    /// Each non-protected shared variable lowered so far and the IR type of its value; for a
    /// package's, a thunk returning the address of its slot (a scalar, or `[data, desc, ..]` as
    /// `VarShape` says). A region's slot address is what its instance cell holds. VHPI reads
    /// and writes the variable there (#309).
    pub fn var_slots(&mut self) -> Result<Vec<VarSlot>, Vec<Diagnostic>> {
        let mut out = Vec::new();
        for (d, ty) in self.cx.shared_vars.clone() {
            let Some(
                Obj::Scalar {
                    space: Space::Shared,
                    off,
                    ..
                }
                | Obj::Array {
                    space: Space::Shared,
                    off,
                },
            ) = self.cx.objs.get(&d).copied()
            else {
                out.push((d, None, ty));
                continue;
            };
            let decl = &self.db.decls[d];
            let (name, span) = (format!("{}'slot", decl.name), decl.span);
            let f = func::thunk(self.db, &mut self.pb, &mut self.cx, &name, span, |fx| {
                Ok(fx.addr(Space::Shared, off))
            });
            self.drain();
            match f {
                Ok(f) if self.cx.diags.is_empty() => out.push((d, Some(f), ty)),
                _ => return Err(self.cx.take_failed()),
            }
        }
        Ok(out)
    }

    /// Lower a subprogram (and everything it calls), returning its function.
    pub fn subprogram(&mut self, decl: DeclId) -> Result<FuncId, Vec<Diagnostic>> {
        let f = func::func_of(self.db, &mut self.pb, &mut self.cx, decl, Span::default());
        self.drain();
        match f {
            Ok(f) if self.cx.diags.is_empty() => Ok(f),
            _ => Err(self.cx.take_failed()),
        }
    }

    /// The IR type of `t` (lowered once), for callers marshalling values of it.
    pub fn ty(&mut self, t: yavs_sem::TypeId) -> Result<yavs_ir::TyId, Vec<Diagnostic>> {
        match ty::lower(self.db, &mut self.cx, &mut self.pb, t, Span::default()) {
            Ok(id) => Ok(id),
            Err(_) => Err(self.cx.take_failed()),
        }
    }

    /// The native of implicit operation `op` (lib/NATIVE.md section 2), for callers running
    /// it on marshalled values (elaboration-time evaluation, #220).
    pub fn implicit_native(
        &mut self,
        op: &yavs_sem::implicit::ImplicitOp,
    ) -> Result<yavs_ir::NativeId, Vec<Diagnostic>> {
        let t = self.ty(op.ty)?;
        let sym = yavs_sem::mangle::implicit_native(self.db, op);
        Ok(self.pb.native(&sym, Some(t)))
    }

    /// The native of attribute `name` (`yavs.attr.image`, `yavs.attr.value`) on prefix type
    /// `t`, for callers running it on marshalled values (elaboration-time evaluation).
    pub fn attr_native(
        &mut self,
        name: &str,
        t: yavs_sem::TypeId,
    ) -> Result<yavs_ir::NativeId, Vec<Diagnostic>> {
        let t = self.ty(t)?;
        Ok(self.pb.native(name, Some(t)))
    }

    /// Every subprogram body declared in the units' regions (a package body, say).
    pub fn subprograms_of(&mut self, unit: UnitId) -> Result<Vec<FuncId>, Vec<Diagnostic>> {
        let u = &self.db.units[unit];
        let scope = u.scope;
        let mut out = Vec::new();
        let decls: Vec<DeclId> = self.db.scopes[scope]
            .order
            .iter()
            .copied()
            .filter(|&d| matches!(&self.db.decls[d].kind, DeclKind::Subprogram(s) if matches!(s.body, yavs_sem::decl::Body::Hir { .. })))
            .collect();
        for d in decls {
            out.push(self.subprogram(d)?);
        }
        Ok(out)
    }

    fn drain(&mut self) {
        while let Some((d, depth)) = self.cx.queue.pop() {
            // Errors are collected in `cx.diags`.
            if func::subprogram(self.db, &mut self.pb, &mut self.cx, d, depth).is_err()
                && let Some(&f) = self.cx.funcs.get(&d)
            {
                self.cx.context_funcs.push(f);
            }
        }
    }

    /// The program lowered so far, verified, as [`Lowerer::finish`] would return it; lowering
    /// can go on. What is lowered later only adds to it: every id it has keeps its meaning.
    pub fn snapshot(&mut self) -> Result<Lowered, Vec<Diagnostic>> {
        self.drain();
        if !self.cx.diags.is_empty() {
            return Err(self.cx.take_failed());
        }
        match self.pb.snapshot() {
            Ok(program) => Ok(Lowered {
                program,
                units: self.units.clone(),
                packages: self.packages.clone(),
                shared_size: self.cx.shared_size,
            }),
            Err(errs) => Err(errs.iter().map(verify_ice).collect()),
        }
    }

    /// [`Lowerer::snapshot`] without copying the program: it leaves the lowerer, which cannot
    /// lower again until [`Lowerer::restore`] (or [`Suspended::restore`]) gets it back. Only
    /// what was lowered since the last snapshot is verified and numbered again.
    pub fn snapshot_take(&mut self) -> Result<Lowered, Vec<Diagnostic>> {
        self.drain();
        if !self.cx.diags.is_empty() {
            return Err(self.cx.take_failed());
        }
        match self.pb.snapshot_take() {
            Ok(program) => Ok(Lowered {
                program,
                units: self.units.clone(),
                packages: self.packages.clone(),
                shared_size: self.cx.shared_size,
            }),
            Err(errs) => Err(errs.iter().map(verify_ice).collect()),
        }
    }

    /// Put back the program [`Lowerer::snapshot_take`] returned (possibly after the runtime
    /// that ran it handed it back).
    pub fn restore(&mut self, program: Program) {
        self.pb.restore(program);
    }

    /// Set the lowering aside without its `Db`, to go on with [`Lowerer::resume`] (the
    /// elaborator adds functions while it changes the `Db` in between).
    pub fn suspend(self) -> Suspended {
        Suspended {
            pb: self.pb,
            cx: self.cx,
            units: self.units,
            packages: self.packages,
        }
    }

    /// Go on with a suspended lowering. `db` must be the `Db` it started with, possibly grown
    /// since: what is lowered already is not lowered again.
    pub fn resume(db: &'a Db, s: Suspended) -> Self {
        Lowerer {
            db,
            pb: s.pb,
            cx: s.cx,
            units: s.units,
            packages: s.packages,
        }
    }

    /// The lowering state as bytes for [`Lowerer::from_image`], to skip lowering what was
    /// lowered so far (the packages' objects, #407). What is lowered so far is verified and
    /// numbered first (as [`Lowerer::snapshot_take`] does); lowering goes on from here
    /// exactly as it does from a restored image. `None` (and nothing verified) when the
    /// constant pool holds over [`IMAGE_MAX_CONSTS`] bytes: reading that back costs more than
    /// lowering again.
    pub fn image(&mut self) -> Result<Option<Vec<u8>>, Vec<Diagnostic>> {
        self.drain();
        if self.pb.const_bytes() > IMAGE_MAX_CONSTS {
            return Ok(None);
        }
        let p = self.snapshot_take()?;
        self.pb.restore(p.program);
        self.pb
            .verify_now()
            .map_err(|errs| errs.iter().map(verify_ice).collect::<Vec<_>>())?;
        let image = ImageRef {
            opts: self.cx.opts,
            pb: &self.pb,
            cx: &self.cx,
            units: &self.units,
            packages: &self.packages,
        };
        postcard::to_stdvec(&image)
            .map(Some)
            .map_err(|e| vec![Diagnostic::error(INTERNAL, e.to_string())])
    }

    /// Go on from [`Lowerer::image`]'s bytes for `db`, which must be the `Db` (and `opts` the
    /// options) the image was made with. `None` when the bytes do not decode or were made with
    /// other options.
    pub fn from_image(db: &'a Db, opts: LowerOptions, bytes: &yavs_diag::Buffer) -> Option<Self> {
        Self::from_image_in(db, opts, bytes, bytes)
    }

    /// [`Lowerer::from_image`] for the image `image`, a part of `buffer` (an entry that holds
    /// something else before it): the function bodies stay in `buffer` until a function runs.
    pub fn from_image_in(
        db: &'a Db,
        opts: LowerOptions,
        buffer: &yavs_diag::Buffer,
        image: &[u8],
    ) -> Option<Self> {
        // The function bodies stay in `buffer` until a function runs: the builder goes on from
        // the functions it has (it reads their headers only) and `Program::func` decodes one
        // when it is first called.
        let i: Image = yavs_diag::sharing(buffer, || {
            yavs_ir::lazy_bodies(|| postcard::from_bytes(image).ok())
        })?;
        if i.opts != opts {
            return None;
        }
        Some(Lowerer {
            db,
            pb: i.pb,
            cx: i.cx,
            units: i.units,
            packages: i.packages,
        })
    }

    /// Verify and freeze the program. Verifier errors are internal errors of this crate.
    pub fn finish(mut self) -> Result<Lowered, Vec<Diagnostic>> {
        self.drain();
        if !self.cx.diags.is_empty() {
            return Err(self.cx.diags);
        }
        match self.pb.finish() {
            Ok(program) => Ok(Lowered {
                program,
                units: self.units,
                packages: self.packages,
                shared_size: self.cx.shared_size,
            }),
            Err(errs) => Err(errs.iter().map(verify_ice).collect()),
        }
    }
}

/// Verify what a release build left to the first use of each function, before `program` is
/// stored (#479): a stored program is read back without verification.
pub fn verify_stored(program: &mut Program) -> Result<(), Vec<Diagnostic>> {
    program
        .verify_pending()
        .map_err(|errs| errs.iter().map(verify_ice).collect())
}

fn verify_ice(e: &VerifyError) -> Diagnostic {
    Diagnostic::error(
        INTERNAL,
        format!("internal error: {e}; this is a bug in yavs"),
    )
}

/// The architecture of entity `top` to elaborate: the last one analyzed (LRM default binding
/// as yavs applies it, sem.md Q4).
pub fn top_architecture(db: &Db, top: &str) -> Option<UnitId> {
    let key = top.to_ascii_lowercase();
    db.units
        .iter()
        .filter(|(_, u)| match &u.hir {
            UnitHir::Architecture { entity, .. } => db.units[*entity].name.key.as_str() == key,
            _ => false,
        })
        .map(|(id, _)| id)
        .last()
}

/// The kind of function a subprogram becomes.
pub(crate) fn func_kind(db: &Db, d: DeclId) -> FuncKind {
    match db.decls[d].kind.subprogram().map(|s| s.kind) {
        Some(yavs_sem::decl::SubprogramKind::Procedure) => FuncKind::Procedure,
        _ => FuncKind::Function,
    }
}

fn forget_free_objects(pb: &mut yavs_ir::ProgramBuilder, cx: &mut Cx) {
    let seeds: Vec<FuncId> = cx
        .free_readers
        .iter()
        .map(|&(_, f)| f)
        .chain(cx.region_inits.values().copied())
        .collect();
    let mut tainted: std::collections::HashSet<FuncId> =
        pb.callers_of(&seeds).into_iter().collect();
    tainted.extend(seeds);
    cx.funcs.retain(|_, f| !tainted.contains(f));
    for o in std::mem::take(&mut cx.free_objs) {
        cx.objs.remove(&o.decl);
    }
    cx.free_readers.clear();
    cx.region_inits.clear();
    cx.capture_free = false;
    cx.capture_vars = false;
}
