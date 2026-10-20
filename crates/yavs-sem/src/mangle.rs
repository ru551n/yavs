//! Mangled names (sem.md 5.3) and native symbols (2.10, `lib/NATIVE.md`).
//! Slices S2 (natives, implicit ops) and S4 (instances).
//!
//! Mangled names are long and readable, built from declaration paths with
//! folded keys: `ieee.numeric_std."+"(ieee.numeric_std.unsigned,ieee.numeric_std.unsigned)ieee.numeric_std.unsigned`.

use yavs_syntax::ast;

use crate::db::Db;
use crate::decl::{DeclKind, Designator};
use crate::ids::{ScopeId, SubtypeId, TypeId};
use crate::implicit::ImplicitOp;
use crate::loader::UnitKey;
use crate::types::TypeKind;

/// The mangled name of a declarative region: `<lib>.<unit>` for a design unit
/// (`<lib>.<entity>.<arch>` for an architecture; a package body shares its
/// package's name), then `.<name>` per nested region. A subprogram region
/// uses the subprogram's mangled name, so overloads stay distinct.
pub fn region(db: &Db, scope: ScopeId) -> String {
    let mut out = String::new();
    region_into(db, scope, &mut out);
    out
}

/// [`region`] appended to `out`.
fn region_into(db: &Db, scope: ScopeId, out: &mut String) {
    let s = &db.scopes[scope];
    let Some(owner) = s.owner else {
        if let Some(p) = s.parent {
            region_into(db, p, out);
        }
        return;
    };
    let d = &db.decls[owner];
    match &d.kind {
        DeclKind::Unit(u) => {
            if let Some(m) = db.generic.unit_names.get(u) {
                out.push_str(m.as_str());
                return;
            }
            let u = &db.units[*u];
            out.push_str(db.libs[u.lib].name.key.as_str());
            out.push('.');
            match u.key {
                UnitKey::Primary(p) => out.push_str(p.as_str()),
                UnitKey::Secondary(p, b) if b.as_str() == "body" => out.push_str(p.as_str()),
                UnitKey::Secondary(p, a) => {
                    out.push_str(p.as_str());
                    out.push('.');
                    out.push_str(a.as_str());
                }
            }
        }
        DeclKind::Subprogram(sp) => out.push_str(sp.mangled.as_str()),
        _ => {
            join_into(db, d.scope, d.name.key().as_str(), out);
        }
    }
}

/// `<region of scope>.<name>`, or `<name>` for the empty region, appended to `out`.
fn join_into(db: &Db, scope: ScopeId, name: &str, out: &mut String) {
    let before = out.len();
    region_into(db, scope, out);
    if out.len() > before {
        out.push('.');
    }
    out.push_str(name);
}

/// The mangled name of a type: its region plus its (declared) name; the
/// universal types are `universal_integer` and `universal_real`.
pub fn type_name(db: &Db, ty: TypeId) -> String {
    let mut out = String::new();
    type_name_into(db, ty, &mut out);
    out
}

fn type_name_into(db: &Db, ty: TypeId, out: &mut String) {
    let t = &db.types[ty];
    match t.kind {
        TypeKind::UniversalInteger => out.push_str("universal_integer"),
        TypeKind::UniversalReal => out.push_str("universal_real"),
        _ => join_into(db, db.decls[t.decl].scope, t.name.key.as_str(), out),
    }
}

/// `<region>.<designator>(<param base types>)[<result base type>]` (5.3).
pub fn subprogram(
    db: &Db,
    scope: ScopeId,
    name: &Designator,
    params: &[SubtypeId],
    ret: Option<SubtypeId>,
) -> String {
    let mut out = String::with_capacity(128);
    join_into(db, scope, name.key().as_str(), &mut out);
    out.push('(');
    for (i, &p) in params.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        type_name_into(db, db.base(p), &mut out);
    }
    out.push(')');
    if let Some(r) = ret {
        type_name_into(db, db.base(r), &mut out);
    }
    out
}

/// The native link of an implicit operation: `yavs.implicit.<family>.<type>[#<form>]`.
pub fn implicit_native(db: &Db, op: &ImplicitOp) -> String {
    let mut s = format!(
        "yavs.implicit.{}.{}",
        op.family.as_str(),
        type_name(db, op.ty)
    );
    if let Some(f) = op.form.suffix() {
        s.push('#');
        s.push_str(f);
    }
    s
}

/// The NATIVE.md symbol of an explicit bodiless std subprogram (section 1):
/// `yavs.<package>.[<protected type>.]<name>.<param type marks>[.r_<return type mark>]`,
/// the rule of `tools/gen_native_md.py`. It depends only on the declaration
/// text: type marks as written, lower case, `'` as `_`.
pub fn native_symbol(package: &str, protected: Option<&str>, spec: &ast::SubprogramSpec) -> String {
    let name = match &spec.designator {
        ast::Designator::Ident(i) => word(i.key.as_str()),
        ast::Designator::Operator(op, _) => match op.as_str() {
            "+" => "add".into(),
            "-" => "sub".into(),
            "*" => "mul".into(),
            "/" => "div".into(),
            other => word(&other.to_ascii_lowercase()),
        },
        ast::Designator::Char(c) => word(&c.value.to_string()),
    };
    let mut params = Vec::new();
    for p in spec.params.iter().flatten() {
        if let ast::InterfaceDecl::Object(o) = p {
            let mark = match &o.subtype {
                Some(ast::IncompleteSubtype::Subtype(si)) => type_mark(&si.type_mark),
                _ => "_".into(),
            };
            params.extend(o.names.iter().map(|_| mark.clone()));
        }
    }
    let mut s = format!("yavs.{}.", package.to_ascii_lowercase());
    if let Some(pt) = protected {
        s.push_str(&pt.to_ascii_lowercase());
        s.push('.');
    }
    s.push_str(&name);
    s.push('.');
    if params.is_empty() {
        s.push_str("void");
    } else {
        s.push_str(&params.join("_"));
    }
    if let (ast::SubprogramKind::Function(_), Some(r)) = (spec.kind, &spec.return_type) {
        s.push_str(".r_");
        s.push_str(&type_mark(r));
    }
    s
}

/// Non-word characters become `_` (Python `re.sub(r"\W", "_", name)`).
fn word(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// A type mark as written: `line`, `std.textio.line`, `t'subtype` as `t_subtype`.
fn type_mark(n: &ast::Name) -> String {
    match &n.kind {
        ast::NameKind::Simple(i) => i.key.as_str().to_string(),
        ast::NameKind::Selected { prefix, suffix } => {
            let s = match suffix {
                ast::Suffix::Ident(i) => i.key.as_str().to_string(),
                ast::Suffix::Char(c) => c.value.to_string(),
                ast::Suffix::Operator(op, _) => op.as_str().to_ascii_lowercase(),
                ast::Suffix::All(_) => "all".into(),
            };
            format!("{}.{s}", type_mark(prefix))
        }
        ast::NameKind::Attribute { prefix, attr } => format!("{}_{}", type_mark(prefix), attr.key),
        _ => "_".into(),
    }
}
