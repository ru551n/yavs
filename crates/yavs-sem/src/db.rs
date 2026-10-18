//! The session database: arenas, well-known entities, diagnostics (sem.md 1.1).
//!
//! One `Db` per session. Ids are deterministic for a given input because
//! analysis order is deterministic; they are not stable across inputs.

use std::collections::{HashMap, HashSet};
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
#[derive(Default, Clone, Debug)]
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
#[derive(Clone, Copy, Debug)]
pub struct Poison {
    pub decl: DeclId,
    pub ty: TypeId,
    pub subtype: SubtypeId,
    pub expr: ExprId,
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

pub struct Db {
    /// The design revision `D`: std and ieee are loaded for it.
    pub rev: LangRev,
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
    /// 2019 method aliases `alias m is pv.m;` in a protected type declaration: the alias
    /// to the private variable `pv` whose method it forwards (5.6.2).
    pub(crate) forwarding: HashMap<DeclId, DeclId>,
    /// 2019 named return values (`return r of t`, 4.2.1), by the function's parameter region:
    /// the hidden constant whose descriptor is the call's result subtype (M2 passes it after
    /// the parameters) and the subtype `r` it constrains.
    pub ret_names: HashMap<ScopeId, (DeclId, SubtypeId)>,
    /// (region, attribute key, entity class) closed by `others`/`all` (7.2).
    pub(crate) attr_closed: HashSet<(ScopeId, Symbol, yavs_syntax::token::Kw)>,
    /// Specifications of labels of statements not analyzed yet (attr.rs, 7.2).
    pub(crate) pending_label_attrs: Vec<crate::attr::PendingLabel>,
    pub well_known: WellKnown,
    pub poison: Poison,
    pub(crate) root: ScopeId,
    /// `None`: the library is unknown and that was reported.
    pub(crate) lib_by_key: HashMap<Symbol, Option<LibId>>,
    pub(crate) files: HashMap<PathBuf, FileCache>,
    pub(crate) stack: Vec<UnitCx>,
    #[allow(dead_code)]
    /// Undeclared names already reported, per (unit, key) (8.2).
    pub(crate) undeclared: HashSet<(Option<UnitId>, Symbol)>,
    /// Spans already given `E1007` or `W1008` by rev.rs (once per construct: names are
    /// classified once per resolution pass).
    pub(crate) rev_reported: HashSet<Span>,
    /// Bottom-up candidate sets of the complete context being resolved, by AST node address.
    pub(crate) memo: HashMap<usize, crate::expr::CandSet>,
    /// The same for name nodes.
    pub(crate) name_memo: HashMap<usize, crate::expr::CandSet>,
    /// Units that depend on a failed unit: undeclared names in them are cascades (8.2).
    pub(crate) tainted: HashSet<UnitId>,
    /// The subtype the next complete context's top expression is resolved for
    /// (an object's declared subtype), so an aggregate with `others` knows its
    /// bounds (9.3.3.3). Consumed by `resolve`.
    pub(crate) context_subtype: Option<SubtypeId>,
    /// `(top AST node address, context subtype)` of the complete context being resolved.
    pub(crate) top_context: Option<(usize, SubtypeId)>,
    /// Folds that overflowed in the complete context being resolved (Q5).
    /// `Some(why)` when the operation has no value (division by zero, ...).
    pub(crate) overflow: Vec<(ExprId, Option<&'static str>)>,
    /// Bounds being resolved are required at analysis time (Q5: array type bounds).
    pub(crate) required_ctx: bool,
    /// Generic types of the instance whose port map is being resolved, with their actuals.
    pub(crate) inst_types: Vec<(TypeId, SubtypeId)>,
    /// External names of the current unit already built, by AST node address (built once:
    /// candidates need the subtype, the HIR is the same).
    pub(crate) ext_memo: HashMap<usize, ExprId>,
    /// The type mark as written of a parameter (by its object decl) and of a function's
    /// result (by the subprogram decl), for messages in the user's spelling (8.3).
    pub(crate) written_marks: HashMap<DeclId, Span>,
    /// A type mark may name an incomplete type: the designated subtype of an
    /// access type definition is being analyzed (alloc.rs, 5.4.2).
    pub(crate) incomplete_ok: bool,
    /// Implicit operations waiting for a well-known type of `std.standard` (BOOLEAN, STRING, ...).
    pub(crate) pending_implicit: Vec<crate::implicit::Pending>,
    /// How much of the arenas `validate` already checked.
    pub(crate) validated: crate::validate::Mark,
    /// Processes and subprograms whose statements are being analyzed, innermost last (stmt.rs).
    pub(crate) stmt_owners: Vec<crate::stmt::Owner>,
    /// Configuration specifications (7.3.1) by the declarative region they are in (unit.rs).
    pub config_specs: HashMap<ScopeId, Vec<crate::hir::ComponentConfig>>,
    /// Configuration specifications whose instance labels wait for the statement part.
    pub(crate) pending_specs: Vec<crate::unit::PendingSpec>,
    /// 2019 range record types (`R'RECORD`) by scalar base type, made on first use (attr.rs).
    pub(crate) range_records: HashMap<TypeId, SubtypeId>,
    /// Generic defaults, generic lists and instance remappings (generic.rs).
    pub(crate) generic: crate::generic::GenericState,
    diags: Vec<Diagnostic>,
    errors: usize,
}

// Helpers below are used by the slices being filled in (CONTRIBUTING.md).
#[allow(dead_code)]
impl Db {
    pub fn new(opts: Options) -> Db {
        let mut scopes = Arena::new();
        let root = scopes.push(Scope::new(ScopeKind::Root, None, None));
        let mut decls: Arena<DeclId, Decl> = Arena::new();
        let mut types: Arena<TypeId, TypeDef> = Arena::new();
        let mut subtypes: Arena<SubtypeId, SubtypeDef> = Arena::new();
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
            attr_index: HashMap::new(),
            forwarding: HashMap::new(),
            ret_names: HashMap::new(),
            attr_closed: HashSet::new(),
            pending_label_attrs: Vec::new(),
            well_known: WellKnown::default(),
            poison: Poison {
                decl,
                ty,
                subtype,
                expr,
            },
            root,
            lib_by_key: HashMap::new(),
            files: HashMap::new(),
            stack: Vec::new(),
            undeclared: HashSet::new(),
            rev_reported: HashSet::new(),
            memo: HashMap::new(),
            name_memo: HashMap::new(),
            pending_implicit: Vec::new(),
            validated: Default::default(),
            overflow: Vec::new(),
            required_ctx: false,
            inst_types: Vec::new(),
            ext_memo: HashMap::new(),
            written_marks: HashMap::new(),
            incomplete_ok: false,
            context_subtype: None,
            top_context: None,
            tainted: HashSet::new(),
            stmt_owners: Vec::new(),
            config_specs: HashMap::new(),
            range_records: HashMap::new(),
            pending_specs: Vec::new(),
            generic: Default::default(),
            diags: Vec::new(),
            errors: 0,
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
        f.text[f.text_offset(span.start)..f.text_offset(span.end)].to_string()
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
