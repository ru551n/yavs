//! `entity lib.module` where `module` is a Verilog module (#776): the instance is typed as a
//! component made from the module's recorded VHDL view (`yavs_lib::Interface`): generics for its
//! parameters (their types from the default values) and ports of `std_logic` and
//! `std_logic_vector`, sized by the defaults. The component is declared in a region of its own
//! below the instance's, so nothing else sees it, and carries the library and module it stands
//! for, which elaboration binds to. Generic and port maps are checked as for any component.

use yavs_diag::{Diagnostic, Span};
use yavs_lib::{IfDir, IfExpr, IfRange, IfValue, Interface};
use yavs_syntax::Symbol;
use yavs_syntax::ast::{self, Ident};

use crate::db::{Db, Reported};
use crate::decl::{Decl, DeclKind, Designator};
use crate::hir::InstTarget;
use crate::ids::{LibId, ScopeId};
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

fn expr(e: &IfExpr, span: Span) -> ast::Expr {
    let b = |e: &IfExpr| Box::new(expr(e, span));
    let kind = match e {
        IfExpr::Num(n) => return number(*n, span),
        IfExpr::Param(p) => ast::ExprKind::Name(simple(p, span)),
        IfExpr::Neg(a) => ast::ExprKind::Unary {
            op: ast::UnaryOp::Minus,
            operand: b(a),
        },
        IfExpr::Bin(op, l, r) => ast::ExprKind::Binary {
            op: match op {
                '+' => ast::BinaryOp::Add,
                '-' => ast::BinaryOp::Sub,
                '*' => ast::BinaryOp::Mul,
                '/' => ast::BinaryOp::Div,
                '%' => ast::BinaryOp::Rem,
                _ => ast::BinaryOp::Pow,
            },
            left: b(l),
            right: b(r),
        },
    };
    // Parenthesised, so that the tree is read as it was built.
    ast::Expr {
        kind: ast::ExprKind::Paren(Box::new(ast::Expr { kind, span })),
        span,
    }
}

/// The range of a port: the bounds as expressions of the generics when the module's bounds
/// were, else as numbers.
fn port_range(r: &IfRange, span: Span) -> ast::SubtypeIndication {
    let mut t = vector(r.msb, r.lsb, span);
    if let (
        Some((m, l)),
        Some(ast::Constraint::Array {
            indexes: Some(ix), ..
        }),
    ) = (&r.exprs, &mut t.constraint)
        && let Some(ast::DiscreteRange::Range(ast::Range::Simple { left, right, .. })) =
            ix.first_mut()
    {
        *left = expr(m, span);
        *right = expr(l, span);
    }
    t
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
    name: &str,
    mode: ast::Mode,
    subtype: ast::SubtypeIndication,
    default: Option<ast::Expr>,
    span: Span,
) -> ast::InterfaceDecl {
    ast::InterfaceDecl::Object(ast::InterfaceObject {
        class: Some(class),
        names: vec![id(name, span)],
        mode: Some(mode),
        subtype: Some(ast::IncompleteSubtype::Subtype(subtype)),
        bus: false,
        default,
        span,
    })
}

/// The component declaration for the module's view; every name is placed at `span`.
fn component(name: &str, view: &Interface, span: Span) -> Result<ast::ComponentDecl, String> {
    let mut generics = Vec::new();
    for p in &view.params {
        let (ty, default) = match &p.value {
            IfValue::Int(i) => (plain(simple("integer", span), span), number(*i, span)),
            IfValue::Real(r) => {
                if !r.is_finite() {
                    return Err(format!("the parameter `{}` is not a finite real", p.name));
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
            &p.name,
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
            Some(r) => port_range(r, span),
        };
        let mode = match p.dir {
            IfDir::In => ast::Mode::In,
            IfDir::Out => ast::Mode::Out,
            IfDir::Inout => ast::Mode::Inout,
        };
        ports.push(object(
            ast::InterfaceClass::Signal,
            &p.name,
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

impl Db {
    /// Verilog modules `modules` (name as written, interface text) were analysed into library
    /// `work` in this session; `entity work.module` finds them before the index of the library
    /// does.
    pub fn note_verilog_modules(&mut self, work: &str, modules: Vec<(String, Option<String>)>) {
        let key = Symbol::intern(&yavs_lib::fold(work));
        self.verilog_modules.entry(key).or_default().extend(modules);
    }

    /// The module `name` of library `lib`, noted for the analysed-unit cache.
    fn find_module(&self, lib: LibId, name: &str) -> Found {
        let key = self.libs[lib].name.key;
        let extra = self
            .verilog_modules
            .get(&key)
            .map_or(&[][..], Vec::as_slice);
        let found = find(Some(&self.libs[lib].source), extra, name);
        self.note_query(
            crate::units::Query::Module(key, Symbol::intern(name)),
            crate::units::Answer::Name(digest(&found)),
        );
        found
    }

    /// `entity lib.module [(arch)]` when the name is a Verilog module and no VHDL entity: the
    /// component standing for it. `None` when the name is not that (an entity, or no module).
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
        let found = self.find_module(lib, module.text.as_str());
        let lib_text = self.libs[lib].name.text;
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
        let comp = match component(&key, &view, span) {
            Ok(c) => c,
            Err(why) => {
                return Some(Err(self.error(
                    Diagnostic::error(
                        "E4703",
                        format!("the Verilog module `{key}` has no VHDL view: {why}"),
                    )
                    .with_primary(span, "instantiated here"),
                )));
            }
        };
        Some(self.module_component(scope, lib, &key, &comp, span))
    }

    /// Declares `comp` in a region below `scope` that sees library `ieee` and returns the
    /// instance target for it.
    fn module_component(
        &mut self,
        scope: ScopeId,
        lib: LibId,
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
        let library = self.libs[lib].name.key.to_string();
        let DeclKind::Component(x) = &mut self.decls[c].kind else {
            return Err(self.ice(span, "the component of a Verilog module is not a component"));
        };
        x.verilog = Some(Box::new((library, key.to_string())));
        Ok(InstTarget::Component(c))
    }
}
