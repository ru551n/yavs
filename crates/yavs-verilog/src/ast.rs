//! The syntax tree of a Verilog source file. Positions are offsets into the preprocessed text of the
//! file; [`crate::pre::Pre::span`] maps them back to the original source.

use crate::pre::Nettype;
use crate::{Dir, Timescale};

pub type Pos = u32;

#[derive(Clone, Debug)]
pub struct Ident {
    pub name: String,
    pub pos: Pos,
}

#[derive(Clone, Debug)]
pub struct Range {
    pub msb: Expr,
    pub lsb: Expr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangeKind {
    Colon,
    PlusColon,
    MinusColon,
}

#[derive(Clone, Debug)]
pub enum Expr {
    /// A missing argument, as in `$display(,)` or a timing check with skipped arguments.
    Empty,
    /// Integer or based literal, as written.
    Num(String),
    Real(String),
    /// A string without its quotes (escapes unprocessed).
    Str(String),
    Ident(Ident),
    /// `a.b`: hierarchical name or member select.
    Dot(Box<Expr>, Ident),
    Index(Box<Expr>, Box<Expr>),
    Range(Box<Expr>, RangeKind, Box<Expr>, Box<Expr>),
    Concat(Vec<Expr>),
    /// `{n{a, b}}`
    Repl(Box<Expr>, Vec<Expr>),
    Unary(&'static str, Box<Expr>),
    Binary(&'static str, Box<Expr>, Box<Expr>),
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    Call(Box<Expr>, Vec<Expr>),
    /// `$name(args)`; the name includes the `$`.
    SysCall(String, Vec<Expr>),
    MinTypMax(Box<Expr>, Box<Expr>, Box<Expr>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetKind {
    Wire,
    Tri,
    Tri0,
    Tri1,
    Wand,
    Triand,
    Wor,
    Trior,
    Trireg,
    Uwire,
    Supply0,
    Supply1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VarKind {
    Reg,
    Integer,
    Time,
    Real,
    Realtime,
}

/// Drive strength pair `(strong0, weak1)` or charge strength `(small)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strength {
    Drive(&'static str, &'static str),
    Charge(&'static str),
}

/// `#d`, `#(d1, d2, d3)`: one to three delay values.
#[derive(Clone, Debug)]
pub struct Delay(pub Vec<Expr>);

#[derive(Clone, Debug)]
pub struct Declarator {
    pub name: Ident,
    /// Unpacked dimensions (memories).
    pub dims: Vec<Range>,
    pub init: Option<Expr>,
}

#[derive(Clone, Debug)]
pub struct PortDecl {
    pub dir: Dir,
    pub net: Option<NetKind>,
    pub var: Option<VarKind>,
    pub signed: bool,
    pub range: Option<Range>,
    pub names: Vec<Declarator>,
}

#[derive(Clone, Debug)]
pub struct NetDecl {
    pub kind: NetKind,
    pub strength: Option<Strength>,
    pub vectored: Option<bool>,
    pub signed: bool,
    pub range: Option<Range>,
    pub delay: Option<Delay>,
    pub names: Vec<Declarator>,
}

#[derive(Clone, Debug)]
pub struct VarDecl {
    pub kind: VarKind,
    pub signed: bool,
    pub range: Option<Range>,
    pub names: Vec<Declarator>,
}

#[derive(Clone, Debug)]
pub struct ParamDecl {
    pub local: bool,
    pub spec: bool,
    pub kind: Option<VarKind>,
    pub signed: bool,
    pub range: Option<Range>,
    pub assigns: Vec<(Ident, Expr)>,
}

#[derive(Clone, Debug)]
pub struct ContAssign {
    pub strength: Option<Strength>,
    pub delay: Option<Delay>,
    pub assigns: Vec<(Expr, Expr)>,
}

#[derive(Clone, Debug)]
pub struct GateInst {
    pub gate: &'static str,
    pub strength: Option<Strength>,
    pub delay: Option<Delay>,
    pub insts: Vec<Instance>,
}

#[derive(Clone, Debug)]
pub struct Instance {
    pub name: Option<Ident>,
    pub range: Option<Range>,
    pub conns: Vec<Conn>,
}

#[derive(Clone, Debug)]
pub enum Conn {
    Ordered(Expr),
    Named(Ident, Option<Expr>),
}

#[derive(Clone, Debug)]
pub struct ModInst {
    pub module: Ident,
    pub params: Vec<Conn>,
    pub insts: Vec<Instance>,
}

#[derive(Clone, Debug)]
pub struct EventExpr {
    pub edge: Option<&'static str>,
    pub expr: Expr,
}

#[derive(Clone, Debug)]
pub enum EventCtrl {
    Star,
    List(Vec<EventExpr>),
}

#[derive(Clone, Debug)]
pub enum Ctrl {
    Delay(Delay),
    Event(EventCtrl),
    /// `repeat (n) @(...)`
    Repeat(Expr, EventCtrl),
}

#[derive(Clone, Debug)]
pub struct CaseItem {
    /// Empty for `default`.
    pub exprs: Vec<Expr>,
    pub stmt: Stmt,
}

#[derive(Clone, Debug)]
pub enum Stmt {
    Null,
    Block {
        par: bool,
        label: Option<Ident>,
        decls: Vec<Item>,
        stmts: Vec<Stmt>,
    },
    Assign {
        lhs: Expr,
        rhs: Expr,
        nonblocking: bool,
        ctrl: Option<Ctrl>,
    },
    /// `assign`, `deassign`, `force`, `release`
    ProcAssign {
        kind: &'static str,
        lhs: Expr,
        rhs: Option<Expr>,
    },
    If {
        cond: Expr,
        then: Box<Stmt>,
        els: Option<Box<Stmt>>,
    },
    Case {
        kind: &'static str,
        expr: Expr,
        items: Vec<CaseItem>,
    },
    For {
        init: (Expr, Expr),
        cond: Expr,
        step: (Expr, Expr),
        body: Box<Stmt>,
    },
    While {
        cond: Expr,
        body: Box<Stmt>,
    },
    Repeat {
        count: Expr,
        body: Box<Stmt>,
    },
    Forever(Box<Stmt>),
    Wait {
        cond: Expr,
        body: Box<Stmt>,
    },
    Delay {
        delay: Delay,
        body: Box<Stmt>,
    },
    Event {
        ctrl: EventCtrl,
        body: Box<Stmt>,
    },
    Trigger(Expr),
    Disable(Expr),
    TaskCall {
        name: Expr,
        args: Vec<Expr>,
    },
    SysCall {
        name: String,
        args: Vec<Expr>,
    },
}

#[derive(Clone, Debug)]
pub struct Task {
    pub automatic: bool,
    pub name: Ident,
    pub decls: Vec<Item>,
    pub body: Vec<Stmt>,
}

#[derive(Clone, Debug)]
pub struct Function {
    pub automatic: bool,
    pub name: Ident,
    pub signed: bool,
    pub range: Option<Range>,
    pub ret: Option<VarKind>,
    pub decls: Vec<Item>,
    pub body: Vec<Stmt>,
}

#[derive(Clone, Debug)]
pub struct GenBody {
    pub label: Option<Ident>,
    pub items: Vec<Item>,
}

#[derive(Clone, Debug)]
pub struct PathDecl {
    /// `if (cond)`
    pub cond: Option<Expr>,
    pub ifnone: bool,
    pub edge: Option<&'static str>,
    pub src: Vec<Expr>,
    /// `*>` (full) instead of `=>` (parallel)
    pub full: bool,
    pub polarity: Option<char>,
    pub dst: Vec<Expr>,
    /// The data source of an edge-sensitive path.
    pub data: Option<Expr>,
    pub delays: Vec<Expr>,
}

#[derive(Clone, Debug)]
pub struct TcArg {
    /// `posedge`, `negedge` or `edge[...]` as written
    pub edge: Option<String>,
    pub expr: Expr,
    pub cond: Option<Expr>,
}

#[derive(Clone, Debug)]
pub enum SpecItem {
    Param(ParamDecl),
    Path(PathDecl),
    Check {
        name: String,
        args: Vec<TcArg>,
    },
    /// `pulsestyle_onevent`, `pulsestyle_ondetect`, `showcancelled`, `noshowcancelled`
    Pulse {
        kind: &'static str,
        outputs: Vec<Expr>,
    },
}

#[derive(Clone, Debug)]
pub enum Item {
    Port(PortDecl),
    Net(NetDecl),
    Var(VarDecl),
    Event(Vec<Declarator>),
    Genvar(Vec<Ident>),
    Param(ParamDecl),
    Assign(ContAssign),
    Gate(GateInst),
    Inst(ModInst),
    Defparam(Vec<(Expr, Expr)>),
    Always(Stmt),
    Initial(Stmt),
    Task(Task),
    Function(Function),
    Generate(Vec<Item>),
    GenIf {
        cond: Expr,
        then: GenBody,
        els: Option<GenBody>,
    },
    GenFor {
        init: (Ident, Expr),
        cond: Expr,
        step: (Ident, Expr),
        body: GenBody,
    },
    GenCase {
        expr: Expr,
        items: Vec<(Vec<Expr>, GenBody)>,
    },
    Specify(Vec<SpecItem>),
}

#[derive(Clone, Debug)]
pub struct NonAnsiPort {
    /// `None` for a concatenation port `{a, b}`.
    pub name: Option<Ident>,
    /// Present for `.name(expr)` and concatenations.
    pub expr: Option<Expr>,
}

#[derive(Clone, Debug)]
pub enum Ports {
    None,
    Ansi(Vec<PortDecl>),
    NonAnsi(Vec<NonAnsiPort>),
}

#[derive(Clone, Debug)]
pub struct Module {
    pub name: Ident,
    pub params: Vec<ParamDecl>,
    pub ports: Ports,
    pub items: Vec<Item>,
    pub timescale: Option<Timescale>,
    pub nettype: Nettype,
    pub celldefine: bool,
    /// The body is a protected envelope (index into the file's envelopes).
    pub encrypted: Option<u32>,
}

/// One row of a UDP table. Symbols are as written: `0 1 x ? b - r f p n *`, or an edge `(01)` as the two letters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UdpRow {
    pub inputs: Vec<UdpSym>,
    pub state: Option<UdpSym>,
    pub output: UdpSym,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UdpSym {
    Level(char),
    Edge(char, char),
}

#[derive(Clone, Debug)]
pub struct Udp {
    pub name: Ident,
    pub output: Ident,
    pub inputs: Vec<Ident>,
    pub sequential: bool,
    pub init: Option<Expr>,
    pub rows: Vec<UdpRow>,
}

#[derive(Clone, Debug)]
pub enum Decl {
    Module(Module),
    Udp(Udp),
    /// A protected envelope outside any module: nothing can be recorded.
    Envelope(u32),
}
