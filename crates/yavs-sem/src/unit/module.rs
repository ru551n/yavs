//! `entity lib.module` where `module` is a Verilog module (#776): the instance is typed as a
//! component made from the module's recorded VHDL view (`yavs_lib::Interface`): generics for its
//! parameters (their types from the default values) and ports of `std_logic` and
//! `std_logic_vector`, sized by the defaults. The component is declared in a region of its own
//! below the instance's, so nothing else sees it, and carries the library and module it stands
//! for, which elaboration binds to. Generic and port maps are checked as for any component.

use yavs_diag::{Diagnostic, Span};
use yavs_lib::{IfBounds, IfDir, IfExpr, IfRange, IfValue, Interface};
use yavs_syntax::Symbol;
use yavs_syntax::ast::{self, Ident};

use crate::db::{Db, Reported};
use crate::decl::{Decl, DeclKind, Designator};
use crate::hir::InstTarget;
use crate::ids::ScopeId;
use crate::loader::LibSource;
use crate::scope::{ScopeKind, Visible};

/// A module lookup in one library.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Found {
    /// The module's name as recorded and its interface text.
    One(String, Option<String>),
    /// Several modules differ from the name only in case.
    Ambiguous(Vec<String>),
    Missing,
}

/// The module `name` of a library: the name as written first, then the one module that differs
/// only in case (D5), among the modules of its index `src` and those analysed since (`extra`).
pub(crate) fn find(
    src: Option<&LibSource>,
    extra: &[(String, Option<String>)],
    name: &str,
) -> Found {
    if let Some((k, i)) = extra.iter().rev().find(|(k, _)| k == name) {
        return Found::One(k.clone(), i.clone());
    }
    let near: Vec<&(String, Option<String>)> = extra
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case(name))
        .collect();
    if let [(k, i)] = near.as_slice() {
        return Found::One(k.clone(), i.clone());
    }
    if near.len() > 1 {
        return Found::Ambiguous(near.iter().map(|(k, _)| k.clone()).collect());
    }
    let Some(LibSource::Manifest(m)) = src else {
        return Found::Missing;
    };
    match m.find_module(name) {
        yavs_lib::ModuleLookup::Found(u) => {
            let key = u.key.to_string();
            let iface = m.module_iface(&key).ok().flatten();
            Found::One(key, iface)
        }
        yavs_lib::ModuleLookup::Ambiguous(v) => {
            Found::Ambiguous(v.into_iter().map(String::from).collect())
        }
        yavs_lib::ModuleLookup::Missing => Found::Missing,
    }
}

/// What a query about module `name` answers, for the analysed-unit cache: nothing, or the
/// recorded name and interface.
pub(crate) fn digest(f: &Found) -> Option<Symbol> {
    match f {
        Found::Missing => None,
        Found::One(k, i) => Some(Symbol::intern(&format!(
            "{k}\n{}",
            i.as_deref().unwrap_or("-")
        ))),
        Found::Ambiguous(v) => Some(Symbol::intern(&format!("?{}", v.join(",")))),
    }
}

fn id(text: &str, span: Span) -> Ident {
    Ident {
        text: Symbol::intern(text),
        key: Symbol::intern(&yavs_lib::fold(text)),
        span,
    }
}

fn simple(text: &str, span: Span) -> ast::Name {
    ast::Name {
        kind: ast::NameKind::Simple(id(text, span)),
        span,
    }
}

fn selected(prefix: ast::Name, text: &str, span: Span) -> ast::Name {
    ast::Name {
        kind: ast::NameKind::Selected {
            prefix: Box::new(prefix),
            suffix: ast::Suffix::Ident(id(text, span)),
        },
        span,
    }
}

/// `ieee.std_logic_1164.NAME`.
fn ieee(text: &str, span: Span) -> ast::Name {
    selected(
        selected(simple("ieee", span), "std_logic_1164", span),
        text,
        span,
    )
}

fn literal(kind: ast::LiteralKind, span: Span) -> ast::Expr {
    ast::Expr {
        kind: ast::ExprKind::Literal(ast::Literal { kind, span }),
        span,
    }
}

fn number(n: i64, span: Span) -> ast::Expr {
    let e = literal(
        ast::LiteralKind::Abstract(Symbol::intern(&n.unsigned_abs().to_string())),
        span,
    );
    match n < 0 {
        true => ast::Expr {
            kind: ast::ExprKind::Unary {
                op: ast::UnaryOp::Minus,
                operand: Box::new(e),
            },
            span,
        },
        false => e,
    }
}

/// What the instance's generic map says, for the bounds of ports.
pub(crate) struct Mapping<'a> {
    /// Some generic is mapped (or the map is positional): a port bound the view cannot follow
    /// may then not be what the defaults give.
    pub any: bool,
    /// The generics the map names, by key.
    pub named: &'a [Symbol],
    /// A parameter that is left out takes its default expression, which names others.
    pub derived: &'a [(&'a str, &'a IfExpr)],
    /// Names (folded) that two names of the module have: VHDL cannot tell them apart by case,
    /// so they are extended identifiers, `\\D\\` and `\\d\\`.
    pub collide: &'a [String],
}

/// The identifier of a formal of the component: the name, or an extended identifier when it
/// differs from another name of the module only in case.
fn formal_ident(name: &str, span: Span, m: &Mapping<'_>) -> Ident {
    if m.collide.contains(&yavs_lib::fold(name)) {
        let e = Symbol::intern(&format!("\\{name}\\"));
        return Ident {
            text: e,
            key: e,
            span,
        };
    }
    id(name, span)
}

impl Mapping<'_> {
    fn mapped(&self, p: &str) -> bool {
        self.any
            && (self.named.is_empty()
                || self
                    .named
                    .contains(&formal_ident(p, Span::default(), self).key))
    }
}

fn paren(kind: ast::ExprKind, span: Span) -> ast::Expr {
    ast::Expr {
        kind: ast::ExprKind::Paren(Box::new(ast::Expr { kind, span })),
        span,
    }
}

fn binary(op: ast::BinaryOp, l: ast::Expr, r: ast::Expr, span: Span) -> ast::Expr {
    paren(
        ast::ExprKind::Binary {
            op,
            left: Box::new(l),
            right: Box::new(r),
        },
        span,
    )
}

/// `boolean'pos(e)`: 1 for true, 0 for false.
fn pos(e: ast::Expr, span: Span) -> ast::Expr {
    let attr = ast::Name {
        kind: ast::NameKind::Attribute {
            prefix: Box::new(simple("boolean", span)),
            attr: id("pos", span),
        },
        span,
    };
    ast::Expr {
        kind: ast::ExprKind::Name(ast::Name {
            kind: ast::NameKind::Apply {
                prefix: Box::new(attr),
                args: vec![ast::AssocElem {
                    formal: None,
                    actual: ast::Actual::Expr(e),
                    span,
                }],
            },
            span,
        }),
        span,
    }
}

/// `e /= 0`
fn truth(e: ast::Expr, span: Span) -> ast::Expr {
    binary(ast::BinaryOp::Ne, e, number(0, span), span)
}

/// The VHDL expression for `e`, an integer: the operators VHDL has, and for the others the
/// arithmetic of `boolean'pos`: a comparison is 0 or 1, `c ? a : b` is `b + (a - b) * [c]`, and
/// `$clog2(x)` counts the powers of two below `x`.
fn expr(e: &IfExpr, span: Span, m: &Mapping<'_>, depth: u8) -> ast::Expr {
    use ast::BinaryOp as B;
    let go = |e: &IfExpr| expr(e, span, m, depth + 1);
    match e {
        IfExpr::Num(n) => number(*n, span),
        IfExpr::Param(p) => match m.derived.iter().find(|(n, _)| n == p) {
            // The instance leaves the parameter out: it is what its default says of the others.
            Some((_, d)) if !m.mapped(p) && depth < 8 => go(d),
            _ => ast::Expr {
                kind: ast::ExprKind::Name(ast::Name {
                    kind: ast::NameKind::Simple(formal_ident(p, span, m)),
                    span,
                }),
                span,
            },
        },
        IfExpr::Neg(a) => paren(
            ast::ExprKind::Unary {
                op: ast::UnaryOp::Minus,
                operand: Box::new(go(a)),
            },
            span,
        ),
        IfExpr::Not(a) => pos(binary(B::Eq, go(a), number(0, span), span), span),
        IfExpr::Clog2(x) => {
            // The number of k in 0 to 30 with 2**k < x.
            let x = go(x);
            let mut sum: Option<ast::Expr> = None;
            for k in 0..31 {
                let below = binary(
                    B::Lt,
                    binary(B::Pow, number(2, span), number(k, span), span),
                    clone_expr(&x, span),
                    span,
                );
                let t = pos(below, span);
                sum = Some(match sum {
                    None => t,
                    Some(s) => binary(B::Add, s, t, span),
                });
            }
            sum.unwrap_or_else(|| number(0, span))
        }
        IfExpr::Bin(op, l, r) => {
            let (l, r) = (go(l), go(r));
            match *op {
                "+" => binary(B::Add, l, r, span),
                "-" => binary(B::Sub, l, r, span),
                "*" => binary(B::Mul, l, r, span),
                "/" => binary(B::Div, l, r, span),
                "%" => binary(B::Rem, l, r, span),
                "**" => binary(B::Pow, l, r, span),
                "<<" => binary(B::Mul, l, binary(B::Pow, number(2, span), r, span), span),
                ">>" => binary(B::Div, l, binary(B::Pow, number(2, span), r, span), span),
                "&&" => pos(binary(B::And, truth(l, span), truth(r, span), span), span),
                "||" => pos(binary(B::Or, truth(l, span), truth(r, span), span), span),
                cmp => {
                    let op = match cmp {
                        "<" => B::Lt,
                        "<=" => B::Le,
                        ">" => B::Gt,
                        ">=" => B::Ge,
                        "==" => B::Eq,
                        _ => B::Ne,
                    };
                    pos(binary(op, l, r, span), span)
                }
            }
        }
        IfExpr::Cond(c, a, b) => {
            let (c, a, b) = (go(c), go(a), go(b));
            let b2 = clone_expr(&b, span);
            binary(
                B::Add,
                b,
                binary(
                    B::Mul,
                    binary(B::Sub, a, b2, span),
                    pos(truth(c, span), span),
                    span,
                ),
                span,
            )
        }
    }
}

/// A copy of an expression `expr` built (names, numbers, the operators above).
fn clone_expr(e: &ast::Expr, span: Span) -> ast::Expr {
    use ast::ExprKind as K;
    let kind = match &e.kind {
        K::Name(n) => K::Name(clone_name(n, span)),
        K::Literal(l) => K::Literal(ast::Literal {
            kind: match &l.kind {
                ast::LiteralKind::Abstract(s) => ast::LiteralKind::Abstract(*s),
                _ => ast::LiteralKind::Abstract(Symbol::intern("0")),
            },
            span,
        }),
        K::Paren(a) => K::Paren(Box::new(clone_expr(a, span))),
        K::Unary { op, operand } => K::Unary {
            op: *op,
            operand: Box::new(clone_expr(operand, span)),
        },
        K::Binary { op, left, right } => K::Binary {
            op: *op,
            left: Box::new(clone_expr(left, span)),
            right: Box::new(clone_expr(right, span)),
        },
        _ => K::Literal(ast::Literal {
            kind: ast::LiteralKind::Abstract(Symbol::intern("0")),
            span,
        }),
    };
    ast::Expr { kind, span }
}

fn clone_name(n: &ast::Name, span: Span) -> ast::Name {
    let kind = match &n.kind {
        ast::NameKind::Simple(i) => ast::NameKind::Simple(*i),
        ast::NameKind::Attribute { prefix, attr } => ast::NameKind::Attribute {
            prefix: Box::new(clone_name(prefix, span)),
            attr: *attr,
        },
        ast::NameKind::Apply { prefix, args } => ast::NameKind::Apply {
            prefix: Box::new(clone_name(prefix, span)),
            args: args
                .iter()
                .map(|a| ast::AssocElem {
                    formal: None,
                    actual: match &a.actual {
                        ast::Actual::Expr(e) => ast::Actual::Expr(clone_expr(e, span)),
                        _ => ast::Actual::Open(span),
                    },
                    span,
                })
                .collect(),
        },
        _ => ast::NameKind::Simple(id("integer", span)),
    };
    ast::Name { kind, span }
}

/// The range of a port: the bounds as expressions of the generics when the module's bounds
/// were, else as numbers; why not when a mapped generic could change bounds the view cannot follow.
fn port_range(r: &IfRange, span: Span, m: &Mapping<'_>) -> Result<ast::SubtypeIndication, String> {
    let mut t = vector(r.msb, r.lsb, span);
    match &r.bounds {
        IfBounds::Fixed => {}
        IfBounds::Opaque(why) if m.any => return Err(why.clone()),
        IfBounds::Opaque(_) => {}
        IfBounds::Exprs(lo, hi) => {
            if let Some(ast::Constraint::Array {
                indexes: Some(ix), ..
            }) = &mut t.constraint
                && let Some(ast::DiscreteRange::Range(ast::Range::Simple { left, right, .. })) =
                    ix.first_mut()
            {
                *left = expr(lo, span, m, 0);
                *right = expr(hi, span, m, 0);
            }
        }
    }
    Ok(t)
}

/// `(msb downto lsb)`, or `(msb to lsb)` for an ascending Verilog range.
fn vector(msb: i64, lsb: i64, span: Span) -> ast::SubtypeIndication {
    let dir = match msb >= lsb {
        true => ast::Direction::Downto,
        false => ast::Direction::To,
    };
    ast::SubtypeIndication {
        resolution: None,
        type_mark: ieee("std_logic_vector", span),
        constraint: Some(ast::Constraint::Array {
            indexes: Some(vec![ast::DiscreteRange::Range(ast::Range::Simple {
                left: number(msb, span),
                dir,
                right: number(lsb, span),
            })]),
            element: None,
            span,
        }),
        span,
    }
}

fn plain(type_mark: ast::Name, span: Span) -> ast::SubtypeIndication {
    ast::SubtypeIndication {
        resolution: None,
        type_mark,
        constraint: None,
        span,
    }
}

fn object(
    class: ast::InterfaceClass,
    name: Ident,
    mode: ast::Mode,
    subtype: ast::SubtypeIndication,
    default: Option<ast::Expr>,
    span: Span,
) -> ast::InterfaceDecl {
    ast::InterfaceDecl::Object(ast::InterfaceObject {
        class: Some(class),
        names: vec![name],
        mode: Some(mode),
        subtype: Some(ast::IncompleteSubtype::Subtype(subtype)),
        bus: false,
        default,
        span,
    })
}

/// The component declaration for the module's view; every name is placed at `span`.
fn component(
    name: &str,
    view: &Interface,
    span: Span,
    m: &Mapping<'_>,
) -> Result<ast::ComponentDecl, (Option<String>, String)> {
    let mut generics = Vec::new();
    for p in &view.params {
        let (ty, default) = match &p.value {
            IfValue::Int(i) => (plain(simple("integer", span), span), number(*i, span)),
            IfValue::Real(r) => {
                if !r.is_finite() {
                    return Err((
                        None,
                        format!("the parameter `{}` is not a finite real", p.name),
                    ));
                }
                let mut text = format!("{:?}", r.abs());
                if !text.contains('.') {
                    // `1e300`: a real literal needs a point.
                    text = match text.split_once('e') {
                        Some((m, e)) => format!("{m}.0e{e}"),
                        None => format!("{text}.0"),
                    };
                }
                let lit = literal(ast::LiteralKind::Abstract(Symbol::intern(&text)), span);
                let lit = match r.is_sign_negative() && *r != 0.0 {
                    true => ast::Expr {
                        kind: ast::ExprKind::Unary {
                            op: ast::UnaryOp::Minus,
                            operand: Box::new(lit),
                        },
                        span,
                    },
                    false => lit,
                };
                (plain(simple("real", span), span), lit)
            }
            IfValue::Str(b) => {
                let s: String = b.iter().map(|&c| c as char).collect();
                (
                    plain(simple("string", span), span),
                    literal(ast::LiteralKind::String(Symbol::intern(&s)), span),
                )
            }
            IfValue::Vec(bits) => (
                vector(bits.len() as i64 - 1, 0, span),
                literal(
                    ast::LiteralKind::String(Symbol::intern(&bits.to_ascii_uppercase())),
                    span,
                ),
            ),
        };
        generics.push(object(
            ast::InterfaceClass::Constant,
            formal_ident(&p.name, span, m),
            ast::Mode::In,
            ty,
            Some(default),
            span,
        ));
    }
    let mut ports = Vec::new();
    for p in &view.ports {
        let ty = match &p.range {
            None => plain(ieee("std_logic", span), span),
            Some(r) => port_range(r, span, m).map_err(|why| (Some(p.name.clone()), why))?,
        };
        let mode = match p.dir {
            IfDir::In => ast::Mode::In,
            IfDir::Out => ast::Mode::Out,
            IfDir::Inout => ast::Mode::Inout,
        };
        ports.push(object(
            ast::InterfaceClass::Signal,
            formal_ident(&p.name, span, m),
            mode,
            ty,
            None,
            span,
        ));
    }
    Ok(ast::ComponentDecl {
        name: id(name, span),
        generics: Some(generics),
        ports: Some(ports),
        end_name: None,
        span,
    })
}

/// The generics a generic map names, by key, and whether it maps any (a positional map names none
/// of them and maps all).
pub(crate) fn map_names(gmap: &[ast::AssocElem]) -> (Vec<Symbol>, bool) {
    let mut named = Vec::new();
    for a in gmap {
        match a.formal.as_ref().map(|f| &f.kind) {
            Some(ast::NameKind::Simple(i)) => named.push(i.key),
            Some(ast::NameKind::Apply { prefix, .. }) => {
                if let ast::NameKind::Simple(i) = &prefix.kind {
                    named.push(i.key);
                }
            }
            _ => return (Vec::new(), true),
        }
    }
    let any = !gmap.is_empty();
    (named, any)
}

/// The generic or port as VHDL sees it in the component made from the module.
fn vhdl_param(p: &yavs_lib::IfParam) -> String {
    match &p.value {
        IfValue::Int(i) => format!("{} : integer := {i}", p.name),
        IfValue::Real(r) => format!("{} : real := {r:?}", p.name),
        IfValue::Str(s) => format!("{} : string := \"{}\"", p.name, String::from_utf8_lossy(s)),
        IfValue::Vec(b) => format!(
            "{} : std_logic_vector({} downto 0) := \"{}\"",
            p.name,
            b.len() - 1,
            b.to_ascii_uppercase()
        ),
    }
}

fn vhdl_port(p: &yavs_lib::IfPort) -> String {
    let mode = match p.dir {
        IfDir::In => "in",
        IfDir::Out => "out",
        IfDir::Inout => "inout",
    };
    match &p.range {
        None => format!("{} : {mode} std_logic", p.name),
        Some(r) => {
            let (m, l) = (r.msb, r.lsb);
            let dir = if m >= l { "downto" } else { "to" };
            let follows = match r.bounds {
                IfBounds::Exprs(..) => " (the bounds follow the generics)",
                _ => "",
            };
            format!(
                "{} : {mode} std_logic_vector({m} {dir} {l}){follows}",
                p.name
            )
        }
    }
}

impl Db {
    /// The errors of the maps of an instance of a Verilog module (from diagnostic `first` on) as
    /// errors about its parameters and ports, each with the Verilog declaration and the VHDL one.
    pub(crate) fn wrap_module_errors(
        &mut self,
        first: usize,
        gmap: &[ast::AssocElem],
        pmap: &[ast::AssocElem],
    ) {
        let Some((module, view)) = self.module_view.take() else {
            return;
        };
        for k in first..self.diags.len() {
            let d = &self.diags[k];
            if d.severity != yavs_diag::Severity::Error || d.code.starts_with("E47") {
                continue;
            }
            let Some(at) = d.primary.as_ref().map(|l| l.span) else {
                continue;
            };
            let within = |a: &ast::AssocElem| {
                a.span.file == at.file && a.span.start <= at.start && at.end <= a.span.end
            };
            let hit = |list: &[ast::AssocElem]| {
                list.iter()
                    .enumerate()
                    .find(|(_, a)| within(a))
                    .map(|(i, a)| {
                        let name = match a.formal.as_ref().map(|f| &f.kind) {
                            Some(ast::NameKind::Simple(n)) => Some(n.text.to_string()),
                            Some(ast::NameKind::Apply { prefix, .. }) => match &prefix.kind {
                                ast::NameKind::Simple(n) => Some(n.text.to_string()),
                                _ => None,
                            },
                            _ => None,
                        };
                        (i, name)
                    })
            };
            let generic = hit(gmap);
            let port = hit(pmap);
            let (is_generic, (idx, name)) = match (generic, port) {
                (Some(g), _) => (true, g),
                (None, Some(p)) => (false, p),
                (None, None) => continue,
            };
            // An extended identifier names one spelling; a plain one ignores case.
            let fold = |s: &str| match s.strip_prefix('\\').and_then(|s| s.strip_suffix('\\')) {
                Some(x) => x.to_string(),
                None => yavs_lib::fold(s),
            };
            let same = |decl: &str, written: &str| match written.starts_with('\\') {
                true => decl == fold(written),
                false => yavs_lib::fold(decl) == fold(written),
            };
            let (what, code, verilog, vhdl, all) = if is_generic {
                let p = match &name {
                    Some(n) => view.params.iter().find(|p| same(&p.name, n)),
                    None => view.params.get(idx),
                };
                (
                    "parameter",
                    "E4704",
                    p.map(|p| p.verilog()),
                    p.map(vhdl_param),
                    view.params
                        .iter()
                        .map(|p| p.name.clone())
                        .collect::<Vec<_>>(),
                )
            } else {
                let p = match &name {
                    Some(n) => view.ports.iter().find(|p| same(&p.name, n)),
                    None => view.ports.get(idx),
                };
                (
                    "port",
                    "E4705",
                    p.map(|p| p.verilog()),
                    p.map(vhdl_port),
                    view.ports
                        .iter()
                        .map(|p| p.name.clone())
                        .collect::<Vec<_>>(),
                )
            };
            let shown = name
                .clone()
                .unwrap_or_else(|| format!("number {}", idx + 1));
            let old = &mut self.diags[k];
            old.code = code;
            old.message = format!(
                "{} map of the Verilog module `{module}`, {what} `{shown}`: {}",
                if is_generic { "generic" } else { "port" },
                old.message
            );
            match (verilog, vhdl) {
                (Some(v), Some(h)) => {
                    old.notes.push(format!("Verilog declares: {v}"));
                    old.notes.push(format!("VHDL sees:       {h}"));
                }
                _ => old.notes.push(format!(
                    "the {what}s of `{module}`: {}",
                    if all.is_empty() {
                        "none".into()
                    } else {
                        all.join(", ")
                    }
                )),
            }
        }
    }

    /// Verilog modules `modules` (name as written, interface text) were analysed into library
    /// `work` in this session; `entity work.module` finds them before the index of the library
    /// does.
    pub fn note_verilog_modules(&mut self, work: &str, modules: Vec<(String, Option<String>)>) {
        let key = Symbol::intern(&yavs_lib::fold(work));
        self.verilog_modules.entry(key).or_default().extend(modules);
    }

    /// The module `name` of the library with key `lib`, noted for the analysed-unit cache.
    fn find_module_in_lib(&self, lib: Symbol, name: &str) -> Found {
        let extra = self
            .verilog_modules
            .get(&lib)
            .map_or(&[][..], Vec::as_slice);
        let source = match self.lib_by_key.get(&lib) {
            Some(Some(l)) => None.or(Some(self.libs[*l].source.clone())),
            _ => match self.lib_source(lib.as_str()) {
                Some(Ok(s)) => Some(s),
                _ => None,
            },
        };
        let found = find(source.as_ref(), extra, name);
        self.note_query(
            crate::units::Query::Module(lib, Symbol::intern(name)),
            crate::units::Answer::Name(digest(&found)),
        );
        found
    }

    /// `entity lib.module [(arch)]` when the name is a Verilog module and no VHDL entity: the
    /// component standing for it. `None` when the name is not that (an entity, or no module).
    /// The named library is searched first, then the `-L` libraries.
    pub(crate) fn verilog_target(
        &mut self,
        scope: ScopeId,
        name: &ast::Name,
        arch: Option<Ident>,
    ) -> Option<Result<InstTarget, Reported>> {
        let ast::NameKind::Selected {
            prefix,
            suffix: ast::Suffix::Ident(module),
        } = &name.kind
        else {
            return None;
        };
        let ast::NameKind::Simple(lib_name) = &prefix.kind else {
            return None;
        };
        let Visible::One(d) = self.lookup(scope, lib_name.key) else {
            return None;
        };
        let DeclKind::Library(lib) = self.decls[d].kind else {
            return None;
        };
        if self.has_entity(lib, module.key) {
            return None;
        }
        let named = self.libs[lib].name.key;
        let lib_text = self.libs[lib].name.text.to_string();
        let mut where_ = (named, lib_text.clone());
        let mut found = self.find_module_in_lib(named, module.text.as_str());
        if found == Found::Missing && !self.opts.verilog_libs.is_empty() {
            let list = self.opts.verilog_libs.clone();
            self.note_query(
                crate::units::Query::Search,
                crate::units::Answer::Name(Some(Symbol::intern(&list.join(" ")))),
            );
            let mut searched = vec![lib_text.clone()];
            for l in &list {
                let key = Symbol::intern(&yavs_lib::fold(l));
                if searched.iter().any(|s| yavs_lib::fold(s) == key.as_str()) {
                    continue;
                }
                searched.push(l.to_string());
                let f = self.find_module_in_lib(key, module.text.as_str());
                if f != Found::Missing {
                    (where_, found) = ((key, l.to_string()), f);
                    break;
                }
            }
            if found == Found::Missing {
                return Some(Err(self.error(
                    Diagnostic::error(
                        crate::diag::UNIT_NOT_FOUND,
                        format!(
                            "design unit `{}` not found in library `{lib_text}`, nor as a Verilog module in the libraries of -L",
                            module.text
                        ),
                    )
                    .with_primary(name.span, "no entity or module with this name")
                    .with_note(format!("libraries searched: {}", searched.join(", ")))
                    .with_help("a module is found in the named library first, then in the -L libraries in order"),
                )));
            }
        }
        let (lib_key, lib_text) = where_;
        let (key, iface) = match found {
            Found::Missing => return None,
            Found::Ambiguous(v) => {
                return Some(Err(self.error(
                    Diagnostic::error(
                        "E4509",
                        format!(
                            "`{}` matches several Verilog modules of library `{lib_text}` that differ only in case: {}",
                            module.text,
                            v.join(", ")
                        ),
                    )
                    .with_primary(module.span, "instantiated here")
                    .with_help("name the module exactly as one of them"),
                )));
            }
            Found::One(k, i) => (k, i),
        };
        if let Some(a) = arch {
            return Some(Err(self.error(
                Diagnostic::error(
                    "E4702",
                    format!("`{key}` is a Verilog module and has no architecture"),
                )
                .with_primary(a.span, "architecture named here")
                .with_help(format!("instantiate it as `entity {lib_text}.{key}`")),
            )));
        }
        let view = match iface.as_deref().map(Interface::decode) {
            Some(Some(v)) if v.error.is_none() => v,
            Some(Some(v)) => {
                return Some(Err(self.error(
                    Diagnostic::error(
                        "E4703",
                        format!(
                            "the Verilog module `{key}` has no VHDL view: {}",
                            v.error.unwrap_or_default()
                        ),
                    )
                    .with_primary(name.span, "instantiated here")
                    .with_help("instantiate it through a component declaration of your own"),
                )));
            }
            _ => {
                return Some(Err(self.error(
                    Diagnostic::error(
                        "E4703",
                        format!(
                            "the library record of the Verilog module `{key}` has no VHDL view"
                        ),
                    )
                    .with_primary(name.span, "instantiated here")
                    .with_help("analyze the Verilog file again"),
                )));
            }
        };
        let span = name.span;
        let (named, any) = self.inst_map.clone();
        let derived: Vec<(&str, &IfExpr)> = view
            .params
            .iter()
            .filter_map(|p| Some((p.name.as_str(), p.derived.as_ref()?)))
            .collect();
        let mut seen: Vec<(String, &str)> = Vec::new();
        let mut collide: Vec<String> = Vec::new();
        for n in view
            .params
            .iter()
            .map(|p| p.name.as_str())
            .chain(view.ports.iter().map(|p| p.name.as_str()))
        {
            let f = yavs_lib::fold(n);
            if seen.iter().any(|(k, s)| *k == f && *s != n) {
                collide.push(f.clone());
            }
            seen.push((f, n));
        }
        let mapping = Mapping {
            any,
            named: &named,
            derived: &derived,
            collide: &collide,
        };
        let comp = match component(&key, &view, span, &mapping) {
            Ok(c) => c,
            Err((port, why)) => {
                let d = match port.and_then(|n| view.ports.iter().find(|p| p.name == n)) {
                    Some(p) => Diagnostic::error(
                        "E4706",
                        format!(
                            "the range of the port `{}` of the Verilog module `{key}` follows its parameters in a way VHDL cannot: {why}",
                            p.name
                        ),
                    )
                    .with_primary(name.span, "instantiated here")
                    .with_note(format!("Verilog: {}", p.verilog()))
                    .with_help("leave the generic map out, or instantiate the module through a component declaration of your own"),
                    None => Diagnostic::error(
                        "E4703",
                        format!("the Verilog module `{key}` has no VHDL view: {why}"),
                    )
                    .with_primary(span, "instantiated here"),
                };
                return Some(Err(self.error(d)));
            }
        };
        self.module_view = Some((key.clone(), view));
        Some(self.module_component(scope, lib_key, &key, &comp, span))
    }

    /// Declares `comp` in a region below `scope` that sees library `ieee` and returns the
    /// instance target for it.
    fn module_component(
        &mut self,
        scope: ScopeId,
        lib: Symbol,
        key: &str,
        comp: &ast::ComponentDecl,
        span: Span,
    ) -> Result<InstTarget, Reported> {
        let ieee_lib = self.library(id("ieee", span))?;
        let region = self.add_scope(ScopeKind::Block, Some(scope), None);
        let d = self.decls.push(Decl {
            name: Designator::Ident(id("ieee", span)),
            kind: DeclKind::Library(ieee_lib),
            scope: region,
            span,
            implicit: true,
            since: crate::LangRev::Vhdl1993,
        });
        self.declare(region, d);
        self.component_decl(region, comp)?;
        let Visible::One(c) = self.lookup(region, comp.name.key) else {
            return Err(self.ice(span, "the component of a Verilog module is not declared"));
        };
        let library = lib.to_string();
        let DeclKind::Component(x) = &mut self.decls[c].kind else {
            return Err(self.ice(span, "the component of a Verilog module is not a component"));
        };
        x.verilog = Some(Box::new((library, key.to_string())));
        Ok(InstTarget::Component(c))
    }
}
