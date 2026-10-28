//! The typed HIR that M2 consumes (sem.md section 7).
//!
//! Every name is resolved to an id, every expression carries a subtype and a
//! staticness, every call names its overload, every statement has a `StmtId`.
//! No HIR node keeps a name to be looked up again (3.9). Spans everywhere;
//! identifiers as `Ident` (spelling plus key, rule 8).
//!
//! Invariants are checked by [`crate::validate`].

use yavs_diag::Span;
use yavs_syntax::Symbol;
use yavs_syntax::ast::{ExternalClass, ForceMode, Ident, PathKind};

pub use crate::attr::PredefAttr;
pub use crate::stat::{ConstValue, Staticness};
pub use crate::types::{Direction, RangeC};

use crate::ids::{
    AttrValueId, ConstId, DeclId, ExprId, ExternalId, InstId, ScopeId, StmtId, SubtypeId, UnitId,
    ViewId,
};

// ---------------------------------------------------------------------------
// Expressions (7.1)

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Expr {
    pub kind: ExprKind,
    /// The result subtype; the type is `subtype.base`.
    pub subtype: SubtypeId,
    pub span: Span,
    pub stat: Staticness,
    /// Folded value, a cache: the tree stays so M2 can always emit code (rule 1).
    pub value: Option<ConstId>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub enum ExprKind {
    /// Literals and folded enum literals.
    Const(ConstId),
    /// Object, interface object, generic constant.
    Obj(DeclId),
    External(ExternalId),
    Call {
        callee: DeclId,
        /// Complete and in formal order.
        args: Box<[Arg]>,
        /// Protected type method call: the object (`self`).
        method_of: Option<ExprId>,
        /// Inserted `??` of a condition (3.6).
        implicit_cond: bool,
    },
    Index {
        prefix: ExprId,
        indices: Box<[ExprId]>,
    },
    Slice {
        prefix: ExprId,
        range: RangeC,
    },
    Field {
        prefix: ExprId,
        elem: u32,
    },
    /// `.all` and implicit dereference.
    Deref {
        prefix: ExprId,
    },
    /// 2019: private variable `var` of protected object `object`, the object of a call
    /// through a method alias that forwards to it (5.6.2). Only as `Call::method_of`.
    PrivateVar {
        object: ExprId,
        var: DeclId,
    },
    Attr {
        attr: PredefAttr,
        prefix: AttrPrefix,
        args: Box<[ExprId]>,
    },
    /// User-defined attribute value.
    AttrValue {
        spec: AttrValueId,
    },
    Aggregate {
        assocs: Box<[AggAssoc]>,
        bounds: AggBounds,
    },
    Qualified {
        to: SubtypeId,
        operand: ExprId,
    },
    /// Explicit type conversion.
    Conv {
        to: SubtypeId,
        operand: ExprId,
    },
    /// Universal operand converted to `Expr::subtype` (2.8); M2 never infers conversions.
    ImplicitConv {
        operand: ExprId,
    },
    New {
        init: NewInit,
    },
    Null,
    /// Port/generic map `open`.
    Open,
    /// 2019 conditional expression `a when c else b`.
    Conditional {
        alts: Box<[(ExprId, Option<ExprId>)]>,
    },
    /// Poison. Never reaches M2: errors stop the pipeline.
    Error,
}

/// The prefix of a predefined attribute or a range attribute.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AttrPrefix {
    /// A type mark.
    Subtype(SubtypeId),
    /// An object or value.
    Expr(ExprId),
    /// A named entity (`'simple_name`, `'path_name`, labels, units).
    Entity(DeclId),
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Arg {
    /// Index into the callee's formals.
    pub formal: u16,
    pub actual: Actual,
    /// The port actual was written `inertial`: an expression actual even for a signal name
    /// (LRM 6.5.6.3).
    pub inertial: bool,
}

/// One part of an individual association: its path, actual, formal part, whether the actual
/// was written `inertial` (an expression actual even for a signal name, LRM 6.5.6.3) and, for
/// a part of an `in` port, whether the actual is a conversion `conv(signal)` (6.5.7.1): a
/// `Call` of one argument or a `Conv` whose operand is the signal name.
pub type IndividualPart = (Box<[LspStep]>, ExprId, ExprId, bool, bool);

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub enum Actual {
    Expr(ExprId),
    /// The formal's default.
    Default,
    Open,
    /// Sub-element association: one entry per sub-element path, with its actual and the
    /// formal part as a name of the formal (`x(0)` in `x(0) => tl`).
    Individual(Box<[IndividualPart]>),
    /// A conversion in an association (6.5.7.1): `formal_conv(f) => actual_conv(actual)`,
    /// with the function, type or subtype declaration of each conversion (a type mark naming a
    /// subtype keeps its constraint, which bounds an unconstrained formal, 5.3.2.2).
    Converted {
        formal_conv: Option<DeclId>,
        actual_conv: Option<DeclId>,
        actual: ExprId,
        /// Subprogram calls (`None` in maps): the actual part as an expression of the
        /// formal's type, copied in for `inout`.
        copy_in: Option<ExprId>,
        /// Subprogram calls (`None` in maps): the formal part as an expression of the
        /// actual's type naming the formal, copied back for `out` and `inout`.
        copy_out: Option<ExprId>,
    },
    /// The actual of a generic type in an instance's generic map (6.5.7.2).
    Subtype(SubtypeId),
    /// The actual of a generic subprogram in an instance's generic map.
    Subprogram(DeclId),
    /// The actual of a formal package in an instance's generic map: a package instance.
    Package(InstId),
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct AggAssoc {
    pub choices: Box<[Choice]>,
    pub value: ExprId,
    /// 2008 1-D aggregates: the value is of the aggregate type (a slice-like piece).
    pub slice_piece: bool,
}

/// Bounds of an aggregate's result (3.4), stored so M2 does not recompute them.
#[derive(serde::Serialize, serde::Deserialize)]
pub enum AggBounds {
    /// A fully constrained context subtype gives them.
    Context,
    /// Left = index subtype 'left, direction of the index subtype.
    Positional { count: ExprId },
    /// From the choices; direction of the index subtype.
    Named { low: ExprId, high: ExprId },
    /// 2008 slice-valued elements: runtime sum of lengths.
    Concat,
    /// Record aggregates.
    Record,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub enum NewInit {
    Subtype(SubtypeId),
    Value(ExprId),
}

// ---------------------------------------------------------------------------
// Longest static prefixes (6.9)

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Lsp {
    pub root: ObjRoot,
    pub path: Box<[LspStep]>,
    pub stat: Staticness,
    /// The name it was computed from (driver and sensitivity diagnostics).
    pub span: Span,
}

/// The `pos` of a [`GuardLit`] on a condition of constants.
pub const GUARD_CONST: u32 = u32::MAX;

/// A signal a pure process reads (`Db::pure_comb_reads`, ...) and, when it reads it only under
/// some conditions, those: `None` when it may read it in every activation, else a disjunction of
/// conjunctions of literals, each of which has to hold for an activation to read the signal.
/// With `lsp` `None` it is no read but a condition: what the process does there is not pure
/// (a `report`, a call of a procedure, an edge test), and it is pure for an instance in which
/// the guard cannot hold (a literal of constants and generics that is false there: debug code).
#[derive(Clone, Debug)]
pub struct PureRead {
    pub lsp: Option<Lsp>,
    pub guard: Option<Vec<Vec<GuardLit>>>,
}

/// `x = c` (`eq`) or `x /= c` on a signal name `x`, `pos` the position of the literal `c` in
/// `std_ulogic` numbering (BIT and BOOLEAN are stored with its codes). With `pos` `GUARD_CONST`,
/// `sig` is a boolean expression of constants and generics instead, and the literal says it is
/// true (`eq`) or false: the elaborator evaluates it for each instance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuardLit {
    pub sig: ExprId,
    pub pos: u32,
    pub eq: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ObjRoot {
    Decl(DeclId),
    External(ExternalId),
    ImplicitSignal(DeclId),
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum LspStep {
    Field(u32),
    Index(Box<[ExprId]>),
    Slice(RangeC),
    /// The steps that follow index the array in the index space of this subtype: the name
    /// went through an object alias with a subtype indication (LRM 6.6.2), whose bounds may
    /// differ from those of the aliased name. Elab and lowering map the positions.
    Rebase(SubtypeId),
}

/// Drivers and `all` sensitivity of a process or of a concurrent statement's
/// equivalent process (M2 builds the process, rule 3).
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct ProcessInfo {
    pub drivers: Vec<Lsp>,
    /// For `process (all)`: the signals read (11.3).
    pub reads_all: Option<Vec<Lsp>>,
}

// ---------------------------------------------------------------------------
// Modes and views

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Mode {
    In,
    Out,
    Inout,
    Buffer,
    Linkage,
    /// 2019 `view v` (record) or `view (v) of arr_t` (`array`).
    View {
        view: ViewId,
        array: bool,
    },
}

// ---------------------------------------------------------------------------
// Statements (7.2)

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Stmt {
    pub id: StmtId,
    pub label: Option<Ident>,
    pub span: Span,
    pub kind: StmtKind,
}

/// A signal or variable target: the name and, for signals, its longest static
/// prefix (an aggregate target has one per element).
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Target {
    pub expr: ExprId,
    pub lsps: Box<[Lsp]>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub enum Rhs<T> {
    Simple(T),
    /// The last condition may be absent.
    Conditional(Box<[(T, Option<ExprId>)]>),
    Selected {
        expr: ExprId,
        matching: bool,
        alts: Box<[(T, ChoiceSet)]>,
    },
}

#[derive(serde::Serialize, serde::Deserialize)]
pub enum Waveform {
    Elements(Box<[WaveElem]>),
    Unaffected,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct WaveElem {
    /// `None` is `null` (disconnect a guarded signal).
    pub value: Option<ExprId>,
    pub after: Option<ExprId>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub enum Delay {
    /// No delay mechanism given: inertial without reject limit.
    Default,
    Transport,
    Inertial {
        reject: Option<ExprId>,
    },
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct ChoiceSet {
    pub choices: Box<[Choice]>,
    /// Folded sorted position intervals, when every choice is locally static.
    pub intervals: Option<Box<[(i64, i64)]>>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub enum Choice {
    Expr(ExprId),
    Range(RangeC),
    /// A record element name in an aggregate.
    Element(u32),
    Others,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub enum LoopScheme {
    Forever,
    While(ExprId),
    For { param: DeclId, range: RangeC },
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Call {
    pub callee: DeclId,
    pub args: Box<[Arg]>,
    pub method_of: Option<ExprId>,
    pub span: Span,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub enum Sensitivity {
    None,
    All,
    List(Box<[Lsp]>),
}

#[derive(serde::Serialize, serde::Deserialize)]
pub enum InstTarget {
    Component(DeclId),
    Entity { unit: UnitId, arch: Option<Ident> },
    Configuration(UnitId),
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct GenAlt {
    pub label: Option<Ident>,
    /// `None` for the final `else`.
    pub cond: Option<ExprId>,
    pub scope: ScopeId,
    pub body: Box<[StmtId]>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub enum StmtKind {
    // Sequential.
    Wait {
        on: Box<[Lsp]>,
        until: Option<ExprId>,
        timeout: Option<ExprId>,
    },
    Assert {
        cond: ExprId,
        report: Option<ExprId>,
        severity: Option<ExprId>,
    },
    Report {
        report: ExprId,
        severity: Option<ExprId>,
    },
    SignalAssign {
        target: Target,
        delay: Delay,
        rhs: Rhs<Waveform>,
    },
    Force {
        target: Target,
        mode: Option<ForceMode>,
        rhs: Rhs<ExprId>,
    },
    Release {
        target: Target,
        mode: Option<ForceMode>,
    },
    VarAssign {
        target: Target,
        rhs: Rhs<ExprId>,
    },
    ProcCall(Call),
    If {
        branches: Box<[(ExprId, Box<[StmtId]>)]>,
        else_: Option<Box<[StmtId]>>,
    },
    Case {
        expr: ExprId,
        matching: bool,
        alts: Box<[(ChoiceSet, Box<[StmtId]>)]>,
    },
    Loop {
        scheme: LoopScheme,
        scope: ScopeId,
        body: Box<[StmtId]>,
    },
    Next {
        loop_: StmtId,
        when: Option<ExprId>,
    },
    Exit {
        loop_: StmtId,
        when: Option<ExprId>,
    },
    Return(Option<ExprId>),
    Null,
    /// 2019 sequential block.
    SeqBlock {
        scope: ScopeId,
        body: Box<[StmtId]>,
    },

    // Concurrent (kept as written, H6).
    Process {
        postponed: bool,
        sensitivity: Sensitivity,
        info: ProcessInfo,
        scope: ScopeId,
        body: Box<[StmtId]>,
    },
    Block {
        scope: ScopeId,
        /// The implicit `GUARD` signal.
        guard: Option<DeclId>,
        generics: Box<[DeclId]>,
        generic_map: Box<[Arg]>,
        ports: Box<[DeclId]>,
        port_map: Box<[Arg]>,
        body: Box<[StmtId]>,
    },
    ConcSignalAssign {
        postponed: bool,
        guarded: bool,
        /// The `GUARD` signal a guarded assignment reads (11.6): the implicit one of the
        /// enclosing guarded block, else an explicitly declared signal of that name that is
        /// visible. `None` when not `guarded`.
        guard: Option<DeclId>,
        target: Target,
        delay: Delay,
        rhs: Rhs<Waveform>,
        info: ProcessInfo,
    },
    ConcProcCall {
        postponed: bool,
        call: Call,
        info: ProcessInfo,
    },
    ConcAssert {
        postponed: bool,
        cond: ExprId,
        report: Option<ExprId>,
        severity: Option<ExprId>,
        info: ProcessInfo,
    },
    Instance {
        target: InstTarget,
        /// The region the instance statement is in: `Db::default_binding(scope, component)`
        /// for a component instance without a configuration (7.3.3).
        scope: ScopeId,
        generic_map: Box<[Arg]>,
        port_map: Box<[Arg]>,
    },
    ForGenerate {
        param: DeclId,
        range: RangeC,
        scope: ScopeId,
        body: Box<[StmtId]>,
    },
    IfGenerate {
        alts: Box<[GenAlt]>,
    },
    CaseGenerate {
        expr: ExprId,
        alts: Box<[(ChoiceSet, GenAlt)]>,
    },
}

// ---------------------------------------------------------------------------
// External names (6.1): the path is data, bound by elab (rule 4).

#[derive(serde::Serialize, serde::Deserialize)]
pub struct External {
    pub class: ExternalClass,
    pub path_kind: PathKind,
    /// Path elements with optional generate index.
    pub path: Box<[(Ident, Option<ExprId>)]>,
    pub subtype: SubtypeId,
    pub span: Span,
}

// ---------------------------------------------------------------------------
// Design units (7.2)

/// Kind-specific HIR of a design unit.
#[derive(serde::Serialize, serde::Deserialize)]
pub enum UnitHir {
    /// Not analyzed yet (or its analysis failed).
    Pending,
    Entity {
        generics: Box<[DeclId]>,
        ports: Box<[DeclId]>,
        stmts: Box<[StmtId]>,
    },
    Architecture {
        entity: UnitId,
        stmts: Box<[StmtId]>,
    },
    Package {
        generics: Option<Box<[DeclId]>>,
    },
    PackageBody {
        package: UnitId,
    },
    PackageInst(InstId),
    Context,
    Configuration(BlockConfig),
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct BlockConfig {
    pub target: BlockTarget,
    pub items: Vec<ConfigItem>,
    pub span: Span,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub enum BlockTarget {
    Architecture(UnitId),
    Block(StmtId),
    /// A generate statement, with an optional index, range or alternative label.
    Generate {
        stmt: StmtId,
        select: Option<GenSelect>,
    },
}

#[derive(serde::Serialize, serde::Deserialize)]
pub enum GenSelect {
    Index(ExprId),
    Range(RangeC),
    Alternative(Symbol),
}

#[derive(serde::Serialize, serde::Deserialize)]
pub enum ConfigItem {
    Block(BlockConfig),
    Component(ComponentConfig),
}

#[derive(serde::Serialize, serde::Deserialize)]
pub enum InstanceSel {
    Labels(Box<[StmtId]>),
    All,
    Others,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct ComponentConfig {
    pub instances: InstanceSel,
    pub component: DeclId,
    pub binding: Option<Binding>,
    pub block: Option<BlockConfig>,
    pub span: Span,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Binding {
    pub aspect: Option<EntityAspect>,
    pub generic_map: Box<[Arg]>,
    pub port_map: Box<[Arg]>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub enum EntityAspect {
    Entity { unit: UnitId, arch: Option<Ident> },
    Configuration(UnitId),
    Open,
}
