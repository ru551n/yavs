//! Declarations: the `Decl` entity and its kinds (sem.md 1.3, 2.10, 6.2), and
//! the analysis of declarative items.

use rsim_diag::Span;
use rsim_syntax::Symbol;
use rsim_syntax::ast::{Ident, SignalKind};

use crate::LangRev;
use crate::hir::Mode;
use crate::ids::{
    DeclId, ExprId, InstId, LibId, ScopeId, StmtId, SubtypeId, TypeId, UnitId, ViewId,
};
use crate::implicit::ImplicitOp;
use crate::types::BoundsFrom;

/// The name of a declaration, original spelling kept (rule 8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Designator {
    Ident(Ident),
    /// Operator symbol as written between the quotes (`"AND"`), plus its folded key.
    Operator {
        text: Symbol,
        key: Symbol,
    },
    Char(char),
}

impl Designator {
    /// The folded key the declaration is looked up by. Operators are keyed with
    /// their quotes (`"and"`), characters with theirs (`'a'`), so neither can
    /// collide with an identifier.
    pub fn key(&self) -> Symbol {
        match *self {
            Designator::Ident(i) => i.key,
            Designator::Operator { key, .. } => key,
            Designator::Char(c) => char_key(c),
        }
    }

    pub fn operator(text: &str) -> Self {
        Designator::Operator {
            text: Symbol::intern(text),
            key: operator_key(text),
        }
    }
}

impl std::fmt::Display for Designator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Designator::Ident(i) => write!(f, "{}", i.text),
            Designator::Operator { text, .. } => write!(f, "\"{text}\""),
            Designator::Char(c) => write!(f, "'{c}'"),
        }
    }
}

/// Lookup key of operator symbol `op` (without quotes).
pub fn operator_key(op: &str) -> Symbol {
    Symbol::intern(&format!("\"{}\"", op.to_ascii_lowercase()))
}

/// Lookup key of character literal `c`.
pub fn char_key(c: char) -> Symbol {
    Symbol::intern(&format!("'{c}'"))
}

pub struct Decl {
    pub name: Designator,
    pub kind: DeclKind,
    pub scope: ScopeId,
    pub span: Span,
    /// Implicitly declared (predefined ops, range records, labels, ...).
    pub implicit: bool,
    /// First revision that has this declaration (sem.md 0.1); `V2008` unless std/ieee says otherwise.
    pub since: LangRev,
}

pub enum DeclKind {
    Library(LibId),
    Unit(UnitId),
    Type(TypeId),
    Subtype(SubtypeId),
    /// Constant, signal, variable, shared variable, file, interface objects.
    Object(Object),
    Subprogram(Subprogram),
    EnumLit {
        ty: TypeId,
        pos: u32,
    },
    PhysUnit {
        ty: TypeId,
        index: u32,
    },
    Alias(Alias),
    /// Attribute declaration.
    Attribute {
        ty: SubtypeId,
    },
    Component(Component),
    View(ViewId),
    PackageInst(InstId),
    /// A generic type (its `TypeKind::Generic` carries the class).
    GenericType(TypeId),
    /// Formal generic subprogram (`Subprogram.body == Body::Interface`).
    GenericSubprogram(Subprogram),
    /// Formal generic package.
    GenericPackage(InstId),
    Group {
        template: DeclId,
    },
    GroupTemplate,
    /// Statement labels are implicitly declared.
    Label(StmtId),
    /// Only reachable through selection, never in `Scope::names`.
    Element {
        record: TypeId,
        index: u32,
    },
    /// A declaration whose analysis failed; silent from then on (8.2).
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjClass {
    Constant,
    Signal,
    Variable,
    SharedVariable,
    File,
}

/// What kind of declaration made the object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObjRole {
    Plain,
    /// `constant c : t;` in a package, completed in the body.
    Deferred,
    Generic,
    Port,
    Param,
    LoopParam,
    GenerateParam,
    /// `s'delayed(t)`, `s'stable(t)`, ... (sem.md 6.3).
    ImplicitSignal,
    /// The implicit `GUARD` signal of a guarded block.
    Guard,
}

pub struct Object {
    pub class: ObjClass,
    pub role: ObjRole,
    pub mode: Option<Mode>,
    pub subtype: SubtypeId,
    pub default: Option<ExprId>,
    pub bounds: BoundsFrom,
    pub signal_kind: Option<SignalKind>,
    /// File declarations: `open kind is name`.
    pub file_open: Option<(Option<ExprId>, ExprId)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubprogramKind {
    Function { pure: bool },
    Procedure,
}

pub struct Subprogram {
    pub kind: SubprogramKind,
    pub params: Box<[Param]>,
    pub ret: Option<SubtypeId>,
    /// Uninstantiated subprogram (2008): its generic declarations.
    pub generics: Option<Box<[DeclId]>>,
    /// The region holding generics and parameters (and, later, the body's declarations).
    pub scope: Option<ScopeId>,
    pub body: Body,
    /// Long readable mangled name (sem.md 5.3).
    pub mangled: Symbol,
}

pub struct Param {
    pub name: Ident,
    pub class: ObjClass,
    pub mode: Mode,
    pub subtype: SubtypeId,
    pub default: Option<ExprId>,
    /// The parameter's object declaration; `None` for implicit operations.
    pub decl: Option<DeclId>,
}

pub enum Body {
    /// Spec analyzed, body not yet seen.
    Pending,
    /// VHDL body: declarations in `scope`, statements in order.
    Hir {
        scope: ScopeId,
        stmts: Box<[StmtId]>,
    },
    /// Predefined operation.
    Implicit(ImplicitOp),
    /// Explicit declaration without body in std (NATIVE.md section 1): the native symbol.
    Native(Symbol),
    /// `'FOREIGN` attribute (VHPIDIRECT, M3): the attribute value.
    Foreign(ExprId),
    Instance(InstId),
    /// Formal generic subprogram.
    Interface,
}

pub enum Alias {
    Object {
        /// A static name.
        target: ExprId,
        subtype: Option<SubtypeId>,
    },
    /// Type, subtype, subprogram (with signature), enum literal, unit, ...
    NonObject { target: DeclId },
}

pub struct Component {
    pub scope: ScopeId,
    pub generics: Box<[DeclId]>,
    pub ports: Box<[DeclId]>,
}

impl DeclKind {
    /// Subprograms and enumeration literals are overloadable (sem.md 1.6);
    /// aliases of those are classified by the caller through their target.
    pub fn is_overloadable(&self) -> bool {
        matches!(
            self,
            DeclKind::Subprogram(_) | DeclKind::GenericSubprogram(_) | DeclKind::EnumLit { .. }
        )
    }

    pub fn subprogram(&self) -> Option<&Subprogram> {
        match self {
            DeclKind::Subprogram(s) | DeclKind::GenericSubprogram(s) => Some(s),
            _ => None,
        }
    }
}
