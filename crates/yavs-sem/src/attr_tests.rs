//! Unit tests for the attribute table and user-defined attributes.

use yavs_diag::Span;
use yavs_syntax::ast;

use super::*;
use crate::expr::aggregate::tests::{Fx, add_type, array_st, ident, konst};
use crate::scope::ScopeKind;
use crate::stat::ConstValue;
use crate::types::Direction;

fn tm(st: SubtypeId, full: bool) -> Prefix {
    Prefix {
        form: PrefixForm::TypeMark,
        subtype: Some(st),
        fully_constrained: full,
    }
}

fn obj(st: SubtypeId, signal: bool) -> Prefix {
    Prefix {
        form: PrefixForm::Object { signal },
        subtype: Some(st),
        fully_constrained: true,
    }
}

fn sel(fx: &mut Fx, name: &str, p: &Prefix) -> Result<Option<PredefAttr>, Reported> {
    select(&mut fx.db, ident(name), p, Span::default()).map(|o| o.map(|i| i.attr))
}

#[test]
fn table_is_complete_and_names_round_trip() {
    for i in TABLE {
        assert_eq!(PredefAttr::from_key(Symbol::intern(i.name)), Some(i.attr));
        assert_eq!(i.attr.info().name, i.name);
        assert!(!i.prefixes.is_empty());
    }
    assert_eq!(TABLE.len(), 38);
    assert_eq!(PredefAttr::from_key(Symbol::intern("range_record")), None);
    assert_eq!(PredefAttr::from_key(Symbol::intern("foo")), None);
}

#[test]
fn scalar_and_array_prefixes() {
    let mut fx = Fx::new();
    let (int, bv) = (fx.int_st, fx.bv_st);
    let bv8 = array_st(&mut fx.db, fx.bv, &[(7, Direction::Downto, 0)]);
    assert_eq!(
        sel(&mut fx, "high", &tm(int, true)).unwrap(),
        Some(PredefAttr::High)
    );
    assert_eq!(
        sel(&mut fx, "length", &obj(bv8, false)).unwrap(),
        Some(PredefAttr::Length)
    );
    assert_eq!(
        sel(&mut fx, "range", &tm(bv8, true)).unwrap(),
        Some(PredefAttr::Range)
    );
    assert_eq!(
        sel(&mut fx, "succ", &tm(int, true)).unwrap(),
        Some(PredefAttr::Succ)
    );
    assert_eq!(sel(&mut fx, "nonsense", &tm(int, true)).unwrap(), None);
    // 'range of an unconstrained array type mark, 'pos of an array.
    assert!(sel(&mut fx, "range", &tm(bv, false)).is_err());
    assert!(sel(&mut fx, "pos", &tm(bv, false)).is_err());
    // Signal attributes need a signal.
    assert!(sel(&mut fx, "event", &obj(int, false)).is_err());
    assert_eq!(
        sel(&mut fx, "event", &obj(int, true)).unwrap(),
        Some(PredefAttr::Event)
    );
    assert_eq!(fx.codes(), [diag::UNKNOWN_ATTRIBUTE; 3]);
}

#[test]
fn revision_gates() {
    let mut fx = Fx::new();
    let int = fx.int_st;
    let bv8 = array_st(&mut fx.db, fx.bv, &[(0, Direction::To, 7)]);
    let range = Prefix {
        form: PrefixForm::Range,
        subtype: Some(int),
        fully_constrained: true,
    };
    // 2019-only forms in a 2008 file: E1007.
    for (name, p) in [
        ("image", obj(int, false)),
        ("left", obj(int, false)),
        ("record", range),
        ("value", range),
        ("index", tm(bv8, true)),
        ("reflect", obj(int, false)),
        ("length", tm(int, true)),
    ] {
        assert!(sel(&mut fx, name, &p).is_err(), "{name}");
    }
    assert!(fx.codes().iter().all(|&c| c == diag::NOT_IN_REVISION));
    fx.db.rev = LangRev::Vhdl2019;
    assert_eq!(
        sel(&mut fx, "image", &obj(int, false)).unwrap(),
        Some(PredefAttr::Image)
    );
    assert_eq!(
        sel(&mut fx, "record", &range).unwrap(),
        Some(PredefAttr::Record)
    );
    assert_eq!(
        sel(&mut fx, "value", &range).unwrap(),
        Some(PredefAttr::Value)
    );
    assert_eq!(
        sel(&mut fx, "index", &tm(bv8, true)).unwrap(),
        Some(PredefAttr::Index)
    );
    // 2019 'image of a composite of scalars.
    assert_eq!(
        sel(&mut fx, "image", &tm(bv8, true)).unwrap(),
        Some(PredefAttr::Image)
    );
}

#[test]
fn result_types_and_staticness() {
    let mut fx = Fx::new();
    let bv8 = array_st(&mut fx.db, fx.bv, &[(0, Direction::To, 7)]);
    let p = obj(bv8, false);
    assert_eq!(
        result_subtype(&fx.db, PredefAttr::Left.info(), &p, 0),
        Some(fx.natural)
    );
    let i = tm(fx.int_st, true);
    assert_eq!(
        result_subtype(&fx.db, PredefAttr::Succ.info(), &i, 0),
        Some(fx.int_st)
    );
    let s = obj(fx.bit_st, true);
    assert_eq!(
        result_subtype(&fx.db, PredefAttr::LastValue.info(), &s, 0),
        Some(fx.bit_st)
    );
    assert_eq!(
        param_type(&fx.db, PredefAttr::Image.info(), &i),
        Some(fx.int)
    );
    use Staticness::{Globally, Locally};
    assert_eq!(staticness(PredefAttr::Left.info(), Locally, None), Locally);
    assert_eq!(
        staticness(PredefAttr::Image.info(), Locally, Some(Globally)),
        Globally
    );
    assert_eq!(
        staticness(PredefAttr::Event.info(), Locally, None),
        Staticness::None
    );
    assert_eq!(
        staticness(PredefAttr::PathName.info(), Locally, None),
        Globally
    );
    assert_eq!(
        staticness(PredefAttr::SimpleName.info(), Staticness::None, None),
        Locally
    );
}

#[test]
fn implicit_signals() {
    let mut fx = Fx::new();
    let root = fx.db.root;
    let scope = fx.db.add_scope(ScopeKind::Architecture, Some(root), None);
    let s = konst(&mut fx.db, ConstValue::Enum(0), fx.bit_st);
    let bool_st = fx.bit_st;
    assert!(
        implicit_signal(
            &mut fx.db,
            PredefAttr::Stable,
            true,
            scope,
            s,
            None,
            bool_st,
            Span::default()
        )
        .is_err()
    );
    assert_eq!(fx.codes(), [diag::SIGNAL_ATTR_IN_SUBPROGRAM]);
    let d = implicit_signal(
        &mut fx.db,
        PredefAttr::Delayed,
        false,
        scope,
        s,
        None,
        bool_st,
        Span::default(),
    )
    .unwrap();
    let DeclKind::Object(o) = &fx.db.decls[d].kind else {
        panic!()
    };
    assert_eq!(o.role, ObjRole::ImplicitSignal);
    assert!(matches!(
        fx.db.exprs[o.default.unwrap()].kind,
        ExprKind::Attr {
            attr: PredefAttr::Delayed,
            ..
        }
    ));
    assert!(fx.db.scopes[scope].order.contains(&d));
}

fn spec(fx: &mut Fx, src: &str) -> ast::AttributeSpec {
    let text = format!("package p is {src} end;");
    let file = fx.db.sources.add("t.vhd", text);
    let parsed = yavs_syntax::parse_file(
        file,
        fx.db.sources.get(file),
        &yavs_syntax::Options::default(),
    );
    assert!(parsed.diags.is_empty(), "{:?}", parsed.diags);
    let ast::LibraryUnit::Package(p) = parsed.design.unwrap().units.remove(0).unit else {
        panic!()
    };
    let Some(ast::Decl::AttributeSpec(s)) = p.decls.into_iter().next() else {
        panic!()
    };
    s
}

fn object(fx: &mut Fx, scope: ScopeId, name: &str, class: ObjClass) -> DeclId {
    let d = fx.db.decls.push(Decl {
        name: Designator::Ident(ident(name)),
        kind: DeclKind::Object(Object {
            class,
            role: ObjRole::Plain,
            mode: None,
            subtype: fx.int_st,
            default: None,
            bounds: BoundsFrom::Subtype,
            signal_kind: None,
            file_open: None,
        }),
        scope,
        span: Span::default(),
        implicit: false,
        since: LangRev::Vhdl1993,
    });
    fx.db.declare(scope, d);
    d
}

#[test]
fn user_attributes() {
    let mut fx = Fx::new();
    let root = fx.db.root;
    let scope = fx.db.add_scope(ScopeKind::Package, Some(root), None);
    let a = declare_attribute(&mut fx.db, scope, ident("a"), fx.int_st, Span::default()).unwrap();
    assert_eq!(spec_subtype(&fx.db, a), Some(fx.int_st));
    let x = object(&mut fx, scope, "x", ObjClass::Signal);
    let y = object(&mut fx, scope, "y", ObjClass::Signal);
    let z = object(&mut fx, scope, "z", ObjClass::Signal);
    let c = object(&mut fx, scope, "c", ObjClass::Constant);
    let one = konst(&mut fx.db, ConstValue::Int(1), fx.int_st);
    let mut nosig = |_: &mut Db, _: &[DeclId], _: &ast::Signature| -> Result<DeclId, Reported> {
        panic!("no signature")
    };
    let s = spec(&mut fx, "attribute a of x : signal is 1;");
    let v = specify(&mut fx.db, scope, &s, a, one, &mut nosig).unwrap();
    assert_eq!(user_value(&fx.db, x, Symbol::intern("a")), Some(v[0]));
    // Twice for x; wrong class for c.
    assert!(specify(&mut fx.db, scope, &s, a, one, &mut nosig).is_err());
    let s = spec(&mut fx, "attribute a of c : signal is 1;");
    assert!(specify(&mut fx.db, scope, &s, a, one, &mut nosig).is_err());
    // others: y and z, not x again.
    let s = spec(&mut fx, "attribute a of others : signal is 1;");
    assert_eq!(
        specify(&mut fx.db, scope, &s, a, one, &mut nosig)
            .unwrap()
            .len(),
        2
    );
    assert!(user_value(&fx.db, y, Symbol::intern("a")).is_some());
    assert!(user_value(&fx.db, z, Symbol::intern("a")).is_some());
    // Nothing for signals after others; constants are another class.
    let s = spec(&mut fx, "attribute a of x : signal is 1;");
    assert!(specify(&mut fx.db, scope, &s, a, one, &mut nosig).is_err());
    let s = spec(&mut fx, "attribute a of all : constant is 1;");
    assert_eq!(
        specify(&mut fx.db, scope, &s, a, one, &mut nosig)
            .unwrap()
            .len(),
        1
    );
    assert!(user_value(&fx.db, c, Symbol::intern("a")).is_some());
    assert_eq!(
        fx.codes(),
        [diag::ATTR_TWICE, diag::ENTITY_CLASS, diag::ATTR_TWICE]
    );
}

#[test]
fn attribute_type_restrictions() {
    let mut fx = Fx::new();
    let root = fx.db.root;
    let scope = fx.db.add_scope(ScopeKind::Package, Some(root), None);
    let (_, acc) = add_type(
        &mut fx.db,
        "ptr",
        TypeKind::Access {
            designated: fx.int_st,
        },
        false,
    );
    assert!(declare_attribute(&mut fx.db, scope, ident("p"), acc, Span::default()).is_err());
    assert_eq!(fx.codes(), [diag::ATTR_TYPE]);
}
