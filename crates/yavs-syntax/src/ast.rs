//! The owned syntax tree.
//!
//! Every node carries a [`Span`]. The tree records what the source says, not
//! what it means: wherever the grammar is ambiguous without declarations, the
//! node is the ambiguous form and semantic analysis decides. In particular:
//!
//! - `f(x)`, `a(i)`, `a(1 to 3)` and `t(x)` (call, index, slice, conversion)
//!   are all [`NameKind::Apply`] with an association list.
//! - A bare name in a discrete range is [`Range::Name`]: a type mark or a
//!   range attribute such as `a'range`.
//! - In choices, `x` may be a value or a subtype name ([`Choice::Expr`]).
//! - A labelled `l: p;` concurrent statement without maps is a
//!   [`ConcStmtKind::ProcedureCall`]; it may be a component instantiation.
//! - Signal assignment targets are expressions: a name or an aggregate.

#![allow(clippy::large_enum_variant)]

use yavs_diag::Span;

use crate::Symbol;
use crate::token::Kw;

/// An identifier: original spelling plus the key it is looked up by.
///
/// For basic identifiers `key` is the lower-case form. Extended identifiers
/// (`\Foo\`) are case-sensitive, so their key is the spelling itself,
/// backslashes included.
#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Ident {
    pub text: Symbol,
    pub key: Symbol,
    pub span: Span,
}

impl std::fmt::Debug for Ident {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} @{:?}", self.text, self.span)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CharLit {
    pub value: char,
    pub span: Span,
}

/// The name of a subprogram, alias or attributed entity.
#[derive(Clone, Debug)]
pub enum Designator {
    Ident(Ident),
    /// Operator symbol such as `"+"`; holds the text between the quotes.
    Operator(Symbol, Span),
    Char(CharLit),
}

#[derive(Debug)]
pub struct DesignFile {
    pub units: Vec<DesignUnit>,
}

#[derive(Debug)]
pub struct DesignUnit {
    pub context: Vec<ContextItem>,
    pub unit: LibraryUnit,
    pub span: Span,
}

#[derive(Debug)]
pub enum ContextItem {
    Library { names: Vec<Ident>, span: Span },
    Use(UseClause),
    Context { names: Vec<Name>, span: Span },
}

#[derive(Debug)]
pub struct UseClause {
    pub names: Vec<Name>,
    pub span: Span,
}

#[derive(Debug)]
pub enum LibraryUnit {
    Entity(EntityDecl),
    Architecture(ArchitectureBody),
    Package(PackageDecl),
    PackageBody(PackageBody),
    PackageInstance(PackageInstance),
    Configuration(ConfigurationDecl),
    Context(ContextDecl),
}

#[derive(Debug)]
pub struct EntityDecl {
    pub name: Ident,
    pub generics: Option<Vec<InterfaceDecl>>,
    pub ports: Option<Vec<InterfaceDecl>>,
    pub decls: Vec<Decl>,
    pub stmts: Vec<ConcStmt>,
    pub end_name: Option<Ident>,
    pub span: Span,
}

#[derive(Debug)]
pub struct ArchitectureBody {
    pub name: Ident,
    /// `lib` in `architecture a of lib.e` (LRM 3.3.1: `entity_name` is a name;
    /// sim1, sim2 and sim3 accept a selected name).
    pub entity_lib: Option<Ident>,
    pub entity: Ident,
    pub decls: Vec<Decl>,
    pub stmts: Vec<ConcStmt>,
    pub end_name: Option<Ident>,
    pub span: Span,
}

#[derive(Debug)]
pub struct PackageDecl {
    pub name: Ident,
    pub generics: Option<Vec<InterfaceDecl>>,
    pub generic_map: Option<Vec<AssocElem>>,
    pub decls: Vec<Decl>,
    pub end_name: Option<Ident>,
    pub span: Span,
}

#[derive(Debug)]
pub struct PackageBody {
    pub name: Ident,
    pub decls: Vec<Decl>,
    pub end_name: Option<Ident>,
    pub span: Span,
}

/// `package p is new lib.gp generic map (...);`
#[derive(Debug)]
pub struct PackageInstance {
    pub name: Ident,
    pub package: Name,
    pub generic_map: Option<Vec<AssocElem>>,
    pub span: Span,
}

#[derive(Debug)]
pub struct ContextDecl {
    pub name: Ident,
    pub items: Vec<ContextItem>,
    pub end_name: Option<Ident>,
    pub span: Span,
}

#[derive(Debug)]
pub struct ConfigurationDecl {
    pub name: Ident,
    pub entity: Name,
    pub decls: Vec<Decl>,
    pub block: Option<BlockConfig>,
    pub end_name: Option<Ident>,
    pub span: Span,
}

/// `for spec {use ...} {items} end for;`
#[derive(Debug)]
pub struct BlockConfig {
    /// Architecture name, block label, or generate label with optional
    /// index or alternative (an [`NameKind::Apply`]).
    pub spec: Name,
    pub uses: Vec<UseClause>,
    pub items: Vec<ConfigItem>,
    pub span: Span,
}

#[derive(Debug)]
pub enum ConfigItem {
    Block(BlockConfig),
    Component(ComponentConfig),
}

#[derive(Debug)]
pub struct ComponentConfig {
    pub spec: ComponentSpec,
    pub binding: Option<BindingIndication>,
    pub block: Option<BlockConfig>,
    pub span: Span,
}

/// `inst1, inst2 : comp`, `all : comp`, `others : comp`.
#[derive(Debug)]
pub struct ComponentSpec {
    pub instances: InstanceList,
    pub component: Name,
    pub span: Span,
}

#[derive(Debug)]
pub enum InstanceList {
    Labels(Vec<Ident>),
    Others,
    All,
}

#[derive(Debug)]
pub struct BindingIndication {
    pub entity_aspect: Option<EntityAspect>,
    pub generic_map: Option<Vec<AssocElem>>,
    pub port_map: Option<Vec<AssocElem>>,
    pub span: Span,
}

#[derive(Debug)]
pub enum EntityAspect {
    Entity { name: Name, arch: Option<Ident> },
    Configuration(Name),
    Open,
}

/// Configuration specification in a declarative part:
/// `for u1 : comp use entity work.e;`.
#[derive(Debug)]
pub struct ConfigSpec {
    pub spec: ComponentSpec,
    pub binding: BindingIndication,
    pub span: Span,
}

#[derive(Debug)]
pub enum Decl {
    Type(TypeDecl),
    Subtype(SubtypeDecl),
    Object(ObjectDecl),
    File(FileDecl),
    Alias(AliasDecl),
    Component(ComponentDecl),
    Attribute(AttributeDecl),
    AttributeSpec(AttributeSpec),
    /// Subprogram declaration without a body.
    Subprogram(SubprogramSpec),
    SubprogramBody(SubprogramBody),
    SubprogramInstance(SubprogramInstance),
    Package(PackageDecl),
    PackageBody(PackageBody),
    PackageInstance(PackageInstance),
    Use(UseClause),
    Group(GroupDecl),
    GroupTemplate(GroupTemplateDecl),
    Disconnect(DisconnectSpec),
    ConfigSpec(ConfigSpec),
    View(ViewDecl),
}

#[derive(Debug)]
pub struct TypeDecl {
    pub name: Ident,
    /// `None` for an incomplete type declaration `type t;`.
    pub def: Option<TypeDef>,
    pub span: Span,
}

#[derive(Debug)]
pub enum TypeDef {
    Enum(Vec<EnumLit>),
    /// Integer or floating point type: `range 0 to 7`.
    Range(Range),
    Physical(PhysicalTypeDef),
    Array {
        indexes: ArrayIndexes,
        element: SubtypeIndication,
    },
    Record {
        elements: Vec<ElementDecl>,
        end_name: Option<Ident>,
    },
    Access(SubtypeIndication),
    File(Name),
    Protected(ProtectedDecl),
    ProtectedBody(ProtectedBody),
    /// 2019 `new p.box generic map (...)`: an instance of a generic protected type (5.6.2).
    ProtectedInstance {
        protected: Name,
        generic_map: Option<Vec<AssocElem>>,
    },
}

#[derive(Debug)]
pub enum EnumLit {
    Ident(Ident),
    Char(CharLit),
}

#[derive(Debug)]
pub struct PhysicalTypeDef {
    pub range: Range,
    pub primary: Ident,
    /// `ns = 1000 ps;`
    pub secondary: Vec<(Ident, Expr)>,
    pub end_name: Option<Ident>,
}

#[derive(Debug)]
pub enum ArrayIndexes {
    /// `(natural range <>, ...)`: the index type marks.
    Unconstrained(Vec<Name>),
    Constrained(Vec<DiscreteRange>),
    /// VHDL-2019 generic array with anonymous index types:
    /// `array (type is (<>)) of t`.
    Anonymous(Vec<GenericTypeDef>),
}

#[derive(Debug)]
pub struct ElementDecl {
    pub names: Vec<Ident>,
    pub subtype: SubtypeIndication,
    pub span: Span,
}

#[derive(Debug)]
pub struct ProtectedDecl {
    /// VHDL-2019 protected type generics.
    pub generics: Option<Vec<InterfaceDecl>>,
    pub generic_map: Option<Vec<AssocElem>>,
    pub decls: Vec<Decl>,
    pub end_name: Option<Ident>,
}

#[derive(Debug)]
pub struct ProtectedBody {
    pub decls: Vec<Decl>,
    pub end_name: Option<Ident>,
}

#[derive(Debug)]
pub struct SubtypeDecl {
    pub name: Ident,
    pub subtype: SubtypeIndication,
    pub span: Span,
}

#[derive(Debug)]
pub struct SubtypeIndication {
    pub resolution: Option<Resolution>,
    pub type_mark: Name,
    pub constraint: Option<Constraint>,
    pub span: Span,
}

#[derive(Debug)]
pub enum Resolution {
    /// A resolution function name.
    Function(Name),
    /// `(f)`: resolve the elements of an array with `f`.
    Element(Box<Resolution>),
    /// `(a f, b g)`: per record element.
    Record(Vec<(Ident, Resolution)>),
}

#[derive(Debug)]
pub enum Constraint {
    /// `range l to r`
    Range(Box<Range>),
    /// `(0 to 3)`, `(open)`, optionally followed by an element constraint:
    /// `(0 to 3)(7 downto 0)`. `indexes` is `None` for `open`.
    Array {
        indexes: Option<Vec<DiscreteRange>>,
        element: Option<Box<Constraint>>,
        span: Span,
    },
    /// `(a(7 downto 0), b(0 to 3))`
    Record(Vec<(Ident, Constraint)>, Span),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Direction {
    To,
    Downto,
}

#[derive(Debug)]
pub enum Range {
    Simple {
        left: Expr,
        dir: Direction,
        right: Expr,
    },
    /// A range attribute (`a'range`) or, in a discrete range, a type mark.
    Name(Name),
}

#[derive(Debug)]
pub enum DiscreteRange {
    Range(Range),
    /// `type_mark range l to r` and other subtype indications with a constraint.
    Subtype(SubtypeIndication),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectClass {
    Constant,
    Signal,
    Variable,
    SharedVariable,
}

#[derive(Debug)]
pub struct ObjectDecl {
    pub class: ObjectClass,
    pub names: Vec<Ident>,
    pub subtype: SubtypeIndication,
    pub signal_kind: Option<SignalKind>,
    pub default: Option<Expr>,
    /// VHDL-2019 `private variable` in a protected type declaration.
    pub private: bool,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SignalKind {
    Register,
    Bus,
}

#[derive(Debug)]
pub struct FileDecl {
    pub names: Vec<Ident>,
    pub subtype: SubtypeIndication,
    pub open_kind: Option<Expr>,
    pub file_name: Option<Expr>,
    pub span: Span,
}

#[derive(Debug)]
pub struct AliasDecl {
    pub designator: Designator,
    pub subtype: Option<SubtypeIndication>,
    /// May end in a [`NameKind::Signature`].
    pub name: Name,
    pub span: Span,
}

#[derive(Debug)]
pub struct ComponentDecl {
    pub name: Ident,
    pub generics: Option<Vec<InterfaceDecl>>,
    pub ports: Option<Vec<InterfaceDecl>>,
    pub end_name: Option<Ident>,
    pub span: Span,
}

#[derive(Debug)]
pub struct AttributeDecl {
    pub name: Ident,
    pub type_mark: Name,
    pub span: Span,
}

#[derive(Debug)]
pub struct AttributeSpec {
    pub attr: Ident,
    pub entities: EntityNameList,
    /// The entity class keyword (`signal`, `function`, `label`, ...).
    pub class: Kw,
    /// The span of the entity class keyword.
    pub class_span: Span,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug)]
pub enum EntityNameList {
    Names(Vec<(Designator, Option<Signature>)>),
    Others,
    All,
}

#[derive(Debug)]
pub struct GroupTemplateDecl {
    pub name: Ident,
    /// Entity classes; `true` when followed by `<>`.
    pub classes: Vec<(Kw, bool)>,
    pub span: Span,
}

#[derive(Debug)]
pub struct GroupDecl {
    pub name: Ident,
    pub template: Name,
    pub constituents: Vec<Name>,
    pub span: Span,
}

#[derive(Debug)]
pub struct DisconnectSpec {
    pub signals: SignalList,
    pub type_mark: Name,
    pub after: Expr,
    pub span: Span,
}

#[derive(Debug)]
pub enum SignalList {
    Names(Vec<Name>),
    Others,
    All,
}

#[derive(Debug)]
pub struct ViewDecl {
    pub name: Ident,
    pub subtype: SubtypeIndication,
    pub elements: Vec<ViewElement>,
    pub end_name: Option<Ident>,
    pub span: Span,
}

#[derive(Debug)]
pub struct ViewElement {
    pub names: Vec<Ident>,
    pub mode: Mode,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purity {
    Pure,
    Impure,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubprogramKind {
    Procedure,
    Function(Option<Purity>),
}

#[derive(Debug)]
pub struct SubprogramSpec {
    pub kind: SubprogramKind,
    pub designator: Designator,
    pub generics: Option<Vec<InterfaceDecl>>,
    pub generic_map: Option<Vec<AssocElem>>,
    pub params: Option<Vec<InterfaceDecl>>,
    /// VHDL-2019 `return r of t`: the name of the return value.
    pub return_ident: Option<Ident>,
    pub return_type: Option<Name>,
    pub span: Span,
}

#[derive(Debug)]
pub struct SubprogramBody {
    pub spec: SubprogramSpec,
    pub decls: Vec<Decl>,
    pub stmts: Vec<SeqStmt>,
    pub end_name: Option<Designator>,
    pub span: Span,
}

/// `function f is new g [sig] generic map (...);`
#[derive(Debug)]
pub struct SubprogramInstance {
    pub kind: SubprogramKind,
    pub designator: Designator,
    /// May end in a [`NameKind::Signature`].
    pub subprogram: Name,
    pub generic_map: Option<Vec<AssocElem>>,
    pub span: Span,
}

#[derive(Debug)]
pub struct Signature {
    pub params: Vec<Name>,
    pub ret: Option<Box<Name>>,
    pub span: Span,
}

#[derive(Debug)]
pub enum InterfaceDecl {
    Object(InterfaceObject),
    Type(Box<InterfaceType>),
    Subprogram(Box<InterfaceSubprogram>),
    Package(Box<InterfacePackage>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InterfaceClass {
    Constant,
    Signal,
    Variable,
    File,
}

#[derive(Debug)]
pub struct InterfaceObject {
    pub class: Option<InterfaceClass>,
    pub names: Vec<Ident>,
    pub mode: Option<Mode>,
    /// `None` only for a mode view without `of subtype`.
    pub subtype: Option<IncompleteSubtype>,
    pub bus: bool,
    pub default: Option<Expr>,
    pub span: Span,
}

#[derive(Debug)]
pub enum Mode {
    In,
    Out,
    Inout,
    Buffer,
    Linkage,
    /// VHDL-2019 `view v [of t]`, or `view (v) of t` (`array` is true).
    View {
        name: Name,
        array: bool,
    },
}

/// A subtype indication or a VHDL-2019 anonymous type `type is private`.
#[derive(Debug)]
pub enum IncompleteSubtype {
    Subtype(SubtypeIndication),
    Anonymous(Box<GenericTypeDef>, Span),
}

#[derive(Debug)]
pub struct InterfaceType {
    pub name: Ident,
    /// VHDL-2019 `type t is ...`; `None` for a plain `type t`.
    pub def: Option<GenericTypeDef>,
    pub span: Span,
}

#[derive(Debug)]
pub enum GenericTypeDef {
    /// `is private`
    Private,
    /// `is <>`
    Scalar,
    /// `is (<>)`
    Discrete,
    /// `is range <>`
    Integer,
    /// `is units <>`
    Physical,
    /// `is range <> . <>`
    Floating,
    Array {
        indexes: ArrayIndexes,
        element: IncompleteSubtype,
    },
    Access(IncompleteSubtype),
    File(IncompleteSubtype),
}

#[derive(Debug)]
pub struct InterfaceSubprogram {
    pub spec: SubprogramSpec,
    pub default: Option<SubprogramDefault>,
}

#[derive(Debug)]
pub enum SubprogramDefault {
    Box,
    Name(Name),
}

#[derive(Debug)]
pub struct InterfacePackage {
    pub name: Ident,
    pub package: Name,
    pub generic_map: InterfacePackageMap,
    pub span: Span,
}

#[derive(Debug)]
pub enum InterfacePackageMap {
    /// `generic map (<>)`
    Box,
    /// `generic map (default)`
    Default,
    Assoc(Vec<AssocElem>),
}

/// `[formal =>] actual` in a generic map, port map or name suffix.
#[derive(Debug)]
pub struct AssocElem {
    pub formal: Option<Name>,
    pub actual: Actual,
    pub span: Span,
}

#[derive(Debug)]
pub enum Actual {
    Expr(Expr),
    /// A slice or other range argument: `a(3 downto 0)`.
    Range(Box<Range>),
    /// A subtype with a constraint: `a(natural range 0 to 3)`, or a
    /// generic type actual `t => integer range 0 to 7`.
    Subtype(Box<SubtypeIndication>),
    Open(Span),
    Inertial(Expr),
}

#[derive(Debug)]
pub struct Name {
    pub kind: NameKind,
    pub span: Span,
}

#[derive(Debug)]
pub enum NameKind {
    Simple(Ident),
    /// Operator symbol used as a name: `"+"(a, b)`.
    Operator(Symbol),
    /// Character literal used as a name (an enumeration literal).
    Character(char),
    Selected {
        prefix: Box<Name>,
        suffix: Suffix,
    },
    /// Function call, index, slice or type conversion: `prefix(args)`.
    Apply {
        prefix: Box<Name>,
        args: Vec<AssocElem>,
    },
    /// `prefix'attr`. An argument (`a'length(1)`) is an enclosing `Apply`.
    Attribute {
        prefix: Box<Name>,
        attr: Ident,
    },
    /// VHDL-2019 call of an uninstantiated subprogram:
    /// `f generic map (t)(x)` is an `Apply` of this.
    GenericMap {
        prefix: Box<Name>,
        args: Vec<AssocElem>,
    },
    /// `prefix [t1, t2 return t3]`
    Signature {
        prefix: Box<Name>,
        signature: Signature,
    },
    External(Box<ExternalName>),
}

#[derive(Debug)]
pub enum Suffix {
    Ident(Ident),
    Char(CharLit),
    Operator(Symbol, Span),
    All(Span),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ExternalClass {
    Constant,
    Signal,
    Variable,
}

/// `<< signal .tb.dut.s : std_logic >>`
#[derive(Debug)]
pub struct ExternalName {
    pub class: ExternalClass,
    pub path: ExternalPath,
    pub subtype: SubtypeIndication,
}

#[derive(Debug)]
pub struct ExternalPath {
    pub kind: PathKind,
    pub elements: Vec<PathElement>,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PathKind {
    /// `@lib.pkg.obj`
    Package,
    /// `.top.a.b`
    Absolute,
    /// `^.^.a.b` or `a.b`, with the number of `^` steps.
    Relative(u32),
}

/// A path element; `index` is the generate index in `g(3)`.
#[derive(Debug)]
pub struct PathElement {
    pub name: Ident,
    pub index: Option<Expr>,
}

#[derive(Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug)]
pub enum ExprKind {
    Name(Name),
    Literal(Literal),
    Aggregate(Vec<ElementAssoc>),
    Paren(Box<Expr>),
    /// `t'(x)` or `t'(a, b)`; `operand` is a `Paren` or `Aggregate`.
    Qualified {
        type_mark: Box<Name>,
        operand: Box<Expr>,
    },
    Unary {
        op: UnaryOp,
        operand: Box<Expr>,
    },
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    /// `new t` or `new t'(x)`
    New(Box<Allocator>),
    /// VHDL-2019 conditional expression `a when c else b`; the last
    /// condition may be absent.
    Conditional(Vec<(Expr, Option<Expr>)>),
    /// `unaffected`, as a VHDL-2019 variable assignment value.
    Unaffected,
    /// Placeholder after a syntax error.
    Error,
}

#[derive(Debug)]
pub enum Allocator {
    Subtype(SubtypeIndication),
    Qualified(Expr),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnaryOp {
    Plus,
    Minus,
    Not,
    Abs,
    /// `??`
    Condition,
    // Reduction operators (VHDL-2008).
    And,
    Or,
    Nand,
    Nor,
    Xor,
    Xnor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryOp {
    And,
    Or,
    Nand,
    Nor,
    Xor,
    Xnor,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    MatchEq,
    MatchNe,
    MatchLt,
    MatchLe,
    MatchGt,
    MatchGe,
    Sll,
    Srl,
    Sla,
    Sra,
    Rol,
    Ror,
    Add,
    Sub,
    Concat,
    Mul,
    Div,
    Mod,
    Rem,
    Pow,
}

#[derive(Debug)]
pub struct Literal {
    pub kind: LiteralKind,
    pub span: Span,
}

#[derive(Debug)]
pub enum LiteralKind {
    /// Decimal or based literal, as written.
    Abstract(Symbol),
    Char(char),
    /// Contents without quotes, doubled quotes undone.
    String(Symbol),
    /// As written, such as `8x"FF"`.
    BitString(Symbol),
    /// `10 ns`; a unit without a value is a [`NameKind::Simple`].
    Physical {
        value: Symbol,
        unit: Ident,
    },
    Null,
}

/// One element of an aggregate: `[choices =>] value`.
#[derive(Debug)]
pub struct ElementAssoc {
    /// Empty for a positional element.
    pub choices: Vec<Choice>,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug)]
pub enum Choice {
    /// A value, an element name or a subtype name.
    Expr(Expr),
    Range(DiscreteRange),
    Others(Span),
}

// Statements.

#[derive(Debug)]
pub struct SeqStmt {
    pub label: Option<Ident>,
    pub kind: SeqStmtKind,
    pub span: Span,
}

#[derive(Debug)]
pub enum SeqStmtKind {
    Wait {
        on: Option<Vec<Name>>,
        until: Option<Expr>,
        timeout: Option<Expr>,
    },
    Assert(Assertion),
    Report {
        report: Expr,
        severity: Option<Expr>,
    },
    SignalAssign(SignalAssign),
    VariableAssign {
        target: Expr,
        rhs: Rhs<Expr>,
    },
    ProcedureCall(Name),
    If {
        branches: Vec<(Expr, Vec<SeqStmt>)>,
        else_: Option<Vec<SeqStmt>>,
        end_label: Option<Ident>,
    },
    Case {
        matching: bool,
        expr: Expr,
        alts: Vec<(Vec<Choice>, Vec<SeqStmt>)>,
        end_label: Option<Ident>,
    },
    Loop {
        scheme: Option<LoopScheme>,
        stmts: Vec<SeqStmt>,
        end_label: Option<Ident>,
    },
    Next {
        label: Option<Ident>,
        when: Option<Expr>,
    },
    Exit {
        label: Option<Ident>,
        when: Option<Expr>,
    },
    Return(Option<Expr>),
    /// VHDL-2019 `return when c;` (10.14): the condition.
    ReturnWhen(Expr),
    Null,
    /// VHDL-2019 sequential block.
    Block {
        decls: Vec<Decl>,
        stmts: Vec<SeqStmt>,
        end_label: Option<Ident>,
    },
}

#[derive(Debug)]
pub enum LoopScheme {
    While(Expr),
    For(Ident, DiscreteRange),
}

#[derive(Debug)]
pub struct Assertion {
    pub cond: Expr,
    pub report: Option<Expr>,
    pub severity: Option<Expr>,
}

#[derive(Debug)]
pub struct SignalAssign {
    pub target: Expr,
    /// Concurrent `guarded` assignment.
    pub guarded: bool,
    pub kind: SignalAssignKind,
}

#[derive(Debug)]
pub enum SignalAssignKind {
    Waveform {
        delay: Option<DelayMechanism>,
        rhs: Rhs<Waveform>,
    },
    Force {
        mode: Option<ForceMode>,
        rhs: Rhs<Expr>,
    },
    Release {
        mode: Option<ForceMode>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ForceMode {
    In,
    Out,
}

/// The right-hand side of an assignment.
#[derive(Debug)]
pub enum Rhs<T> {
    Simple(T),
    /// `a when c1 else b when c2 else d`: the last condition may be absent.
    Conditional(Vec<(T, Option<Expr>)>),
    /// `with e select[?] t <= a when c1, b when others;`
    Selected {
        expr: Expr,
        matching: bool,
        alts: Vec<(T, Vec<Choice>)>,
    },
}

#[derive(Debug)]
pub enum Waveform {
    Elements(Vec<WaveformElement>),
    Unaffected(Span),
}

#[derive(Debug)]
pub struct WaveformElement {
    /// `null` is a [`LiteralKind::Null`].
    pub value: Expr,
    pub after: Option<Expr>,
}

#[derive(Debug)]
pub enum DelayMechanism {
    Transport,
    Inertial { reject: Option<Expr> },
}

#[derive(Debug)]
pub struct ConcStmt {
    pub label: Option<Ident>,
    pub postponed: bool,
    pub kind: ConcStmtKind,
    pub span: Span,
}

#[derive(Debug)]
pub enum ConcStmtKind {
    Process(ProcessStmt),
    Block(BlockStmt),
    Instance(InstanceStmt),
    ProcedureCall(Name),
    Assert(Assertion),
    SignalAssign(SignalAssign),
    ForGenerate {
        param: Ident,
        range: DiscreteRange,
        body: GenerateBody,
    },
    IfGenerate {
        /// `(alternative label, condition, body)`
        branches: Vec<(Option<Ident>, Expr, GenerateBody)>,
        else_: Option<(Option<Ident>, GenerateBody)>,
    },
    CaseGenerate {
        expr: Expr,
        alts: Vec<(Option<Ident>, Vec<Choice>, GenerateBody)>,
    },
}

#[derive(Debug)]
pub struct ProcessStmt {
    pub sensitivity: Option<Sensitivity>,
    pub decls: Vec<Decl>,
    pub stmts: Vec<SeqStmt>,
    pub end_label: Option<Ident>,
}

#[derive(Debug)]
pub enum Sensitivity {
    All(Span),
    List(Vec<Name>),
}

#[derive(Debug)]
pub struct BlockStmt {
    pub guard: Option<Expr>,
    pub generics: Option<Vec<InterfaceDecl>>,
    pub generic_map: Option<Vec<AssocElem>>,
    pub ports: Option<Vec<InterfaceDecl>>,
    pub port_map: Option<Vec<AssocElem>>,
    pub decls: Vec<Decl>,
    pub stmts: Vec<ConcStmt>,
    pub end_label: Option<Ident>,
}

#[derive(Debug)]
pub struct InstanceStmt {
    pub unit: InstantiatedUnit,
    pub generic_map: Option<Vec<AssocElem>>,
    pub port_map: Option<Vec<AssocElem>>,
}

#[derive(Debug)]
pub enum InstantiatedUnit {
    /// `comp` or `component comp`
    Component(Name),
    Entity {
        name: Name,
        arch: Option<Ident>,
    },
    Configuration(Name),
}

#[derive(Debug)]
pub struct GenerateBody {
    pub decls: Vec<Decl>,
    pub stmts: Vec<ConcStmt>,
    /// The alternative label after an inner `end`.
    pub end_label: Option<Ident>,
    pub span: Span,
}
