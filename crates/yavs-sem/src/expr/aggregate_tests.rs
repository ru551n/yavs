//! Unit tests for aggregates and choices, on hand-built types with a mock
//! resolver (the real one lives in `resolve.rs`).

use std::collections::HashMap;

use yavs_diag::Span;
use yavs_syntax::Symbol;
use yavs_syntax::ast::{self, Ident};

use super::*;
use crate::decl::{Decl, DeclKind};
use crate::expr::Interp;
use crate::expr::Via;
use crate::types::{RecordElem, TypeDef};
use crate::{LangRev, Options};

pub(crate) fn ident(s: &str) -> Ident {
    Ident {
        text: Symbol::intern(s),
        key: Symbol::intern(&s.to_ascii_lowercase()),
        span: Span::default(),
    }
}

pub(crate) struct Fx {
    pub db: Db,
    pub uint: TypeId,
    pub int: TypeId,
    pub int_st: SubtypeId,
    pub natural: SubtypeId,
    pub bit: TypeId,
    pub bit_st: SubtypeId,
    pub sul: TypeId,
    pub sul_st: SubtypeId,
    pub bv: TypeId,
    pub bv_st: SubtypeId,
    pub enums: Vec<TypeId>,
}

pub(crate) fn add_type(
    db: &mut Db,
    name: &str,
    kind: TypeKind,
    anonymous: bool,
) -> (TypeId, SubtypeId) {
    let ty = db.types.next_id();
    let st = db.subtypes.push(SubtypeDef {
        base: ty,
        name: Some(ident(name)),
        parent: None,
        constraint: Constraint::None,
        resolution: None,
        span: Span::default(),
    });
    let decl = db.decls.push(Decl {
        name: Designator::Ident(ident(name)),
        kind: DeclKind::Type(ty),
        scope: db.root,
        span: Span::default(),
        implicit: false,
        since: LangRev::Vhdl1993,
    });
    db.types.push(TypeDef {
        decl,
        name: ident(name),
        anonymous,
        kind,
        first_subtype: st,
        base_subtype: st,
    });
    (ty, st)
}

pub(crate) fn add_enum(db: &mut Db, name: &str, lits: &[Designator]) -> (TypeId, SubtypeId) {
    let ty = db.types.next_id();
    let decls: Vec<_> = lits
        .iter()
        .enumerate()
        .map(|(i, &d)| {
            db.decls.push(Decl {
                name: d,
                kind: DeclKind::EnumLit { ty, pos: i as u32 },
                scope: db.root,
                span: Span::default(),
                implicit: false,
                since: LangRev::Vhdl1993,
            })
        })
        .collect();
    add_type(db, name, TypeKind::Enum { lits: decls.into() }, false)
}

pub(crate) fn chars(s: &str) -> Vec<Designator> {
    s.chars().map(Designator::Char).collect()
}

pub(crate) fn konst(db: &mut Db, v: ConstValue, st: SubtypeId) -> ExprId {
    mk_const(db, v, st, Span::default())
}

pub(crate) fn range_st(
    db: &mut Db,
    base_st: SubtypeId,
    l: i64,
    dir: Direction,
    r: i64,
) -> SubtypeId {
    let base = db.base(base_st);
    let enum_ = matches!(db.types[base].kind, TypeKind::Enum { .. });
    let v = |p| {
        if enum_ {
            ConstValue::Enum(p as u32)
        } else {
            ConstValue::Int(p)
        }
    };
    let left = konst(db, v(l), base_st);
    let right = konst(db, v(r), base_st);
    db.subtypes.push(SubtypeDef {
        base,
        name: None,
        parent: Some(base_st),
        constraint: Constraint::Range(RangeC::Explicit { left, dir, right }),
        resolution: None,
        span: Span::default(),
    })
}

pub(crate) fn array_st(db: &mut Db, base: TypeId, dims: &[(i64, Direction, i64)]) -> SubtypeId {
    let TypeKind::Array { indexes, .. } = &db.types[base].kind else {
        panic!()
    };
    let indexes = indexes.to_vec();
    let ranges = dims
        .iter()
        .zip(indexes)
        .map(|(&(l, d, r), ist)| {
            let left = konst(db, pos_value(db, db.base(ist), l), ist);
            let right = konst(db, pos_value(db, db.base(ist), r), ist);
            RangeC::Explicit {
                left,
                dir: d,
                right,
            }
        })
        .collect();
    db.subtypes.push(SubtypeDef {
        base,
        name: None,
        parent: None,
        constraint: Constraint::Array {
            dims: Some(ranges),
            elem: Box::new(Constraint::None),
        },
        resolution: None,
        span: Span::default(),
    })
}

impl Fx {
    pub(crate) fn new() -> Fx {
        let mut db = Db::new(Options::new(LangRev::Vhdl2008, crate::default_stdlib()));
        let (uint, _) = add_type(
            &mut db,
            "universal_integer",
            TypeKind::UniversalInteger,
            true,
        );
        db.well_known.universal_integer = Some(uint);
        let (int, int_st) = add_type(&mut db, "integer", TypeKind::Integer, false);
        // INTEGER's first subtype carries the 32-bit range.
        let lo = konst(&mut db, ConstValue::Int(i64::from(i32::MIN)), int_st);
        let hi = konst(&mut db, ConstValue::Int(i64::from(i32::MAX)), int_st);
        db.subtypes[int_st].constraint = Constraint::Range(RangeC::Explicit {
            left: lo,
            dir: Direction::To,
            right: hi,
        });
        db.well_known.integer = Some(int);
        let natural = range_st(&mut db, int_st, 0, Direction::To, i64::from(i32::MAX));
        db.well_known.natural = Some(natural);
        let (bit, bit_st) = add_enum(&mut db, "bit", &chars("01"));
        db.well_known.bit = Some(bit);
        let (sul, sul_st) = add_enum(&mut db, "std_ulogic", &chars("UX01ZWLH-"));
        let (bv, bv_st) = add_type(
            &mut db,
            "bit_vector",
            TypeKind::Array {
                indexes: Box::new([natural]),
                elem: bit_st,
            },
            false,
        );
        Fx {
            db,
            uint,
            int,
            int_st,
            natural,
            bit,
            bit_st,
            sul,
            sul_st,
            bv,
            bv_st,
            enums: vec![bit, sul],
        }
    }

    pub(crate) fn codes(&self) -> Vec<&'static str> {
        self.db.diagnostics().iter().map(|d| d.code).collect()
    }
}

/// Mock resolution: literals, registered objects and type marks.
#[derive(Default)]
pub(crate) struct Mock {
    pub enums: Vec<TypeId>,
    /// Name to (subtype, staticness, value).
    pub objs: HashMap<Symbol, (SubtypeId, Staticness, Option<ConstValue>)>,
    pub marks: HashMap<Symbol, SubtypeId>,
}

impl Mock {
    pub(crate) fn new(fx: &Fx) -> Mock {
        Mock {
            enums: fx.enums.clone(),
            ..Default::default()
        }
    }

    fn mismatch(db: &mut Db, span: Span) -> Reported {
        db.error(
            Diagnostic::error(diag::TYPE_MISMATCH, "mock: type mismatch")
                .with_primary(span, "here"),
        )
    }
}

impl AggResolver for Mock {
    fn cands(&mut self, db: &mut Db, e: &ast::Expr) -> CandSet {
        let ty = |t| Interp::new(t, Via::Other);
        match &e.kind {
            ast::ExprKind::Literal(l) => match &l.kind {
                ast::LiteralKind::Abstract(_) => CandSet::Types(smallvec::smallvec![ty(db
                    .well_known
                    .universal_integer
                    .unwrap())]),
                ast::LiteralKind::Char(c) => CandSet::Types(
                    self.enums
                        .iter()
                        .filter(|&&t| match &db.types[t].kind {
                            TypeKind::Enum { lits } => lits
                                .iter()
                                .any(|&d| db.decls[d].name == Designator::Char(*c)),
                            _ => false,
                        })
                        .map(|&t| ty(t))
                        .collect(),
                ),
                _ => CandSet::Class(ClassMask::STRING_LIKE),
            },
            ast::ExprKind::Aggregate(_) => CandSet::Class(ClassMask::ANY_COMPOSITE),
            ast::ExprKind::Paren(inner) => self.cands(db, inner),
            ast::ExprKind::Name(ast::Name {
                kind: ast::NameKind::Simple(id),
                ..
            }) => match self.objs.get(&id.key) {
                Some(&(st, _, _)) => CandSet::Types(smallvec::smallvec![ty(db.base(st))]),
                None => CandSet::Types(smallvec::smallvec![]),
            },
            _ => CandSet::Types(smallvec::smallvec![]),
        }
    }

    fn resolve(&mut self, db: &mut Db, e: &ast::Expr, exp: Expected) -> Result<ExprId, Reported> {
        let Expected::Type(t) = exp else {
            panic!("mock needs a type")
        };
        let st = db.types[t].first_subtype;
        match &e.kind {
            ast::ExprKind::Paren(inner) => self.resolve(db, inner, exp),
            ast::ExprKind::Literal(l) => match &l.kind {
                ast::LiteralKind::Abstract(s)
                    if db.types[t].kind.is_discrete()
                        && !matches!(db.types[t].kind, TypeKind::Enum { .. }) =>
                {
                    Ok(mk_const(
                        db,
                        ConstValue::Int(s.as_str().parse().unwrap()),
                        st,
                        e.span,
                    ))
                }
                ast::LiteralKind::Char(c) => match &db.types[t].kind {
                    TypeKind::Enum { lits } => match lits
                        .iter()
                        .position(|&d| db.decls[d].name == Designator::Char(*c))
                    {
                        Some(p) => Ok(mk_const(db, ConstValue::Enum(p as u32), st, e.span)),
                        None => Err(Mock::mismatch(db, e.span)),
                    },
                    _ => Err(Mock::mismatch(db, e.span)),
                },
                ast::LiteralKind::String(s) => {
                    let TypeKind::Array { indexes, elem } = &db.types[t].kind else {
                        return Err(Mock::mismatch(db, e.span));
                    };
                    let (ist, et) = (indexes[0], db.base(*elem));
                    let TypeKind::Enum { lits } = &db.types[et].kind else {
                        panic!()
                    };
                    let runs: Vec<_> = s
                        .as_str()
                        .chars()
                        .map(|c| {
                            let p = lits
                                .iter()
                                .position(|&d| db.decls[d].name == Designator::Char(c))
                                .unwrap();
                            (1, ConstValue::Enum(p as u32))
                        })
                        .collect();
                    let ir = subtype_range(db, ist, 0).unwrap();
                    let n = runs.len() as i64;
                    let r = StaticRange::new(
                        ir.left,
                        ir.dir,
                        if ir.dir == Direction::To {
                            ir.left + n - 1
                        } else {
                            ir.left - n + 1
                        },
                    );
                    let v = ConstValue::Array(array_const(db, et, vec![r], runs));
                    Ok(mk_const(db, v, st, e.span))
                }
                _ => Err(Mock::mismatch(db, e.span)),
            },
            ast::ExprKind::Name(ast::Name {
                kind: ast::NameKind::Simple(id),
                ..
            }) => match self.objs.get(&id.key).cloned() {
                Some((ost, stat, v)) if db.base(ost) == t => Ok(mk_expr(
                    db,
                    ExprKind::Obj(db.poison.decl),
                    ost,
                    e.span,
                    stat,
                    v,
                )),
                _ => Err(Mock::mismatch(db, e.span)),
            },
            ast::ExprKind::Aggregate(a) => resolve_aggregate(db, self, a, st, e.span),
            _ => Err(Mock::mismatch(db, e.span)),
        }
    }

    fn discrete_range(
        &mut self,
        db: &mut Db,
        r: &ast::DiscreteRange,
        index_ty: TypeId,
    ) -> Result<RangeC, Reported> {
        match r {
            ast::DiscreteRange::Range(ast::Range::Simple { left, dir, right }) => {
                let left = self.resolve(db, left, Expected::Type(index_ty))?;
                let right = self.resolve(db, right, Expected::Type(index_ty))?;
                Ok(RangeC::Explicit {
                    left,
                    dir: *dir,
                    right,
                })
            }
            _ => panic!("mock: unsupported range"),
        }
    }

    fn type_mark(&mut self, _db: &mut Db, e: &ast::Expr) -> Option<SubtypeId> {
        match &e.kind {
            ast::ExprKind::Name(ast::Name {
                kind: ast::NameKind::Simple(id),
                ..
            }) => self.marks.get(&id.key).copied(),
            _ => None,
        }
    }
}

/// Parse `src` as an expression (the default of a constant).
pub(crate) fn parse_expr(fx: &mut Fx, src: &str) -> ast::Expr {
    let text = format!("package p is constant c : t := {src}; end;");
    let file = fx.db.sources.add("test.vhd", text);
    let parsed = yavs_syntax::parse_file(
        file,
        fx.db.sources.get(file),
        &yavs_syntax::Options::default(),
    );
    assert!(parsed.diags.is_empty(), "{:?}", parsed.diags);
    let mut design = parsed.design.unwrap();
    let ast::LibraryUnit::Package(p) = design.units.remove(0).unit else {
        panic!()
    };
    let Some(ast::Decl::Object(o)) = p.decls.into_iter().next() else {
        panic!()
    };
    o.default.unwrap()
}

fn assocs(e: ast::Expr) -> Vec<ast::ElementAssoc> {
    match e.kind {
        ast::ExprKind::Aggregate(a) => a,
        k => panic!("not an aggregate: {k:?}"),
    }
}

fn agg(fx: &mut Fx, m: &mut Mock, src: &str, ctx: SubtypeId) -> Result<ExprId, Reported> {
    let e = parse_expr(fx, src);
    let span = e.span;
    resolve_aggregate(&mut fx.db, m, &assocs(e), ctx, span)
}

fn bits(fx: &Fx, e: ExprId) -> (Vec<(i64, Direction, i64)>, String) {
    let Some(ConstValue::Array(a)) = fx.db.value(e) else {
        panic!("not folded")
    };
    let s = array_positions(a, 1 << 20)
        .unwrap()
        .iter()
        .map(|&p| char::from(b'0' + p as u8))
        .collect();
    (a.bounds.to_vec(), s)
}

use Direction::{Downto, To};

// -- array aggregates ---------------------------------------------------------

#[test]
fn positional_unconstrained_context_takes_index_left() {
    let mut fx = Fx::new();
    let mut m = Mock::new(&fx);
    let bv = fx.bv_st;
    let e = agg(&mut fx, &mut m, "('1', '0', '1')", bv).unwrap();
    assert_eq!(bits(&fx, e), (vec![(0, To, 2)], "101".into()));
    assert!(matches!(
        fx.db.exprs[e].kind,
        ExprKind::Aggregate {
            bounds: AggBounds::Positional { .. },
            ..
        }
    ));
    assert_eq!(fx.db.exprs[e].stat, Staticness::Locally);
}

#[test]
fn others_in_constrained_context_is_one_run() {
    let mut fx = Fx::new();
    let mut m = Mock::new(&fx);
    let st = array_st(&mut fx.db, fx.bv, &[(65535, Downto, 0)]);
    let e = agg(&mut fx, &mut m, "(others => '1')", st).unwrap();
    let Some(ConstValue::Array(a)) = fx.db.value(e) else {
        panic!()
    };
    assert_eq!(&*a.bounds, &[(65535, Downto, 0)]);
    assert_eq!(elem_runs(a), vec![(65536, ConstValue::Enum(1))]);
    assert!(matches!(
        fx.db.exprs[e].kind,
        ExprKind::Aggregate {
            bounds: AggBounds::Context,
            ..
        }
    ));
    assert_eq!(fx.db.exprs[e].subtype, st);
}

#[test]
fn large_others_stays_runs() {
    let mut fx = Fx::new();
    let mut m = Mock::new(&fx);
    let st = array_st(&mut fx.db, fx.bv, &[(0, To, 1 << 20)]);
    let e = agg(&mut fx, &mut m, "(0 => '1', others => '0')", st).unwrap();
    let Some(ConstValue::Array(a)) = fx.db.value(e) else {
        panic!()
    };
    match &a.elems {
        Elems::Runs(r) => assert_eq!(
            &**r,
            &[(1, ConstValue::Enum(1)), (1 << 20, ConstValue::Enum(0))]
        ),
        Elems::Bytes(_) => panic!("2^20 elements must stay run-length"),
    }
}

#[test]
fn others_downto_named_positions() {
    let mut fx = Fx::new();
    let mut m = Mock::new(&fx);
    let st = array_st(&mut fx.db, fx.bv, &[(7, Downto, 0)]);
    let e = agg(
        &mut fx,
        &mut m,
        "(7 => '1', 1 downto 0 => '1', others => '0')",
        st,
    )
    .unwrap();
    assert_eq!(bits(&fx, e).1, "10000011");
}

#[test]
fn others_without_constrained_context_is_e1325() {
    let mut fx = Fx::new();
    let mut m = Mock::new(&fx);
    let bv = fx.bv_st;
    assert!(agg(&mut fx, &mut m, "(others => '1')", bv).is_err());
    assert_eq!(fx.codes(), [diag::AGG_OTHERS]);
}

#[test]
fn others_not_last_or_not_alone_is_e1325() {
    let mut fx = Fx::new();
    let mut m = Mock::new(&fx);
    let st = array_st(&mut fx.db, fx.bv, &[(0, To, 3)]);
    assert!(agg(&mut fx, &mut m, "(others => '1', 0 => '0')", st).is_err());
    assert!(agg(&mut fx, &mut m, "(0 | others => '1')", st).is_err());
    assert_eq!(fx.codes(), [diag::AGG_OTHERS, diag::AGG_OTHERS]);
}

#[test]
fn mixed_named_positional_is_e1324() {
    let mut fx = Fx::new();
    let mut m = Mock::new(&fx);
    let st = array_st(&mut fx.db, fx.bv, &[(0, To, 3)]);
    assert!(agg(&mut fx, &mut m, "('1', 2 => '0', 3 => '1', 1 => '0')", st).is_err());
    assert_eq!(fx.codes(), [diag::AGG_NONSTATIC_CHOICE]);
    // A trailing others after positional elements is fine.
    let e = agg(&mut fx, &mut m, "('1', '0', others => '1')", st).unwrap();
    assert_eq!(bits(&fx, e).1, "1011");
}

#[test]
fn named_must_be_contiguous_e1328() {
    let mut fx = Fx::new();
    let mut m = Mock::new(&fx);
    let bv = fx.bv_st;
    assert!(agg(&mut fx, &mut m, "(1 => '1', 3 => '0')", bv).is_err());
    assert_eq!(fx.codes(), [diag::CHOICE_UNCOVERED]);
    assert!(
        fx.db.diagnostics()[0]
            .message
            .contains("missing choices for 2"),
        "{}",
        fx.db.diagnostics()[0].message
    );
}

#[test]
fn named_bounds_from_choices() {
    let mut fx = Fx::new();
    let mut m = Mock::new(&fx);
    let bv = fx.bv_st;
    let e = agg(&mut fx, &mut m, "(3 => '1', 1 to 2 => '0')", bv).unwrap();
    assert_eq!(bits(&fx, e), (vec![(1, To, 3)], "001".into()));
    assert!(matches!(
        fx.db.exprs[e].kind,
        ExprKind::Aggregate {
            bounds: AggBounds::Named { .. },
            ..
        }
    ));
    // The result subtype is constrained 1 to 3.
    assert_eq!(
        subtype_range(&fx.db, fx.db.exprs[e].subtype, 0),
        Some(StaticRange::new(1, To, 3))
    );
}

#[test]
fn named_in_downto_context_takes_its_direction() {
    let mut fx = Fx::new();
    let mut m = Mock::new(&fx);
    let st = array_st(&mut fx.db, fx.bv, &[(3, Downto, 0)]);
    let e = agg(&mut fx, &mut m, "(3 => '1', 2 downto 0 => '0')", st).unwrap();
    assert_eq!(bits(&fx, e), (vec![(3, Downto, 0)], "1000".into()));
}

#[test]
fn named_length_mismatch_with_context_is_e1405() {
    let mut fx = Fx::new();
    let mut m = Mock::new(&fx);
    let st = array_st(&mut fx.db, fx.bv, &[(0, To, 3)]);
    // Same length elsewhere is fine (implicit subtype conversion).
    assert!(agg(&mut fx, &mut m, "(4 to 7 => '1')", st).is_ok());
    let positional = agg(&mut fx, &mut m, "('1', '0', '1')", st);
    assert!(positional.is_err());
    assert_eq!(fx.codes(), [diag::LENGTH_MISMATCH]);
}

#[test]
fn overlap_is_e1327() {
    let mut fx = Fx::new();
    let mut m = Mock::new(&fx);
    let st = array_st(&mut fx.db, fx.bv, &[(0, To, 3)]);
    assert!(
        agg(
            &mut fx,
            &mut m,
            "(0 to 2 => '1', 2 => '0', others => '0')",
            st
        )
        .is_err()
    );
    assert_eq!(fx.codes(), [diag::CHOICE_OVERLAP]);
}

#[test]
fn choice_outside_context_with_others_is_e1403() {
    let mut fx = Fx::new();
    let mut m = Mock::new(&fx);
    let st = array_st(&mut fx.db, fx.bv, &[(0, To, 3)]);
    assert!(agg(&mut fx, &mut m, "(5 => '1', others => '0')", st).is_err());
    assert_eq!(fx.codes(), [diag::OUT_OF_SUBTYPE]);
}

#[test]
fn non_static_choice_alone_is_allowed_else_e1326() {
    let mut fx = Fx::new();
    let mut m = Mock::new(&fx);
    m.objs.insert(
        Symbol::intern("g"),
        (fx.natural, Staticness::Globally, None),
    );
    let bv = fx.bv_st;
    let e = agg(&mut fx, &mut m, "(g => '1')", bv).unwrap();
    assert_eq!(fx.db.exprs[e].stat, Staticness::Globally);
    assert!(fx.db.value(e).is_none());
    assert!(matches!(
        fx.db.exprs[e].kind,
        ExprKind::Aggregate {
            bounds: AggBounds::Named { .. },
            ..
        }
    ));
    assert!(agg(&mut fx, &mut m, "(g => '1', 3 => '0')", bv).is_err());
    assert_eq!(fx.codes(), [diag::AGG_NONSTATIC_CHOICE]);
}

#[test]
fn locally_static_unfolded_choice_is_deferred() {
    let mut fx = Fx::new();
    let mut m = Mock::new(&fx);
    // A locally static ieee call sem cannot fold (Q3).
    m.objs
        .insert(Symbol::intern("f"), (fx.natural, Staticness::Locally, None));
    let st = array_st(&mut fx.db, fx.bv, &[(0, To, 3)]);
    let e = agg(&mut fx, &mut m, "(f => '1', 2 => '0', others => '0')", st).unwrap();
    assert!(fx.db.value(e).is_none());
    assert!(fx.codes().is_empty());
}

#[test]
fn slice_pieces_2008() {
    let mut fx = Fx::new();
    let mut m = Mock::new(&fx);
    let bv = fx.bv_st;
    let e = agg(&mut fx, &mut m, "(\"10\", '1', \"011\")", bv).unwrap();
    let ExprKind::Aggregate { bounds, assocs } = &fx.db.exprs[e].kind else {
        panic!()
    };
    assert!(matches!(bounds, AggBounds::Concat));
    assert_eq!(
        assocs.iter().map(|a| a.slice_piece).collect::<Vec<_>>(),
        [true, false, true]
    );
    assert_eq!(bits(&fx, e), (vec![(0, To, 5)], "101011".into()));
    // A range choice may take an array value; lengths must agree.
    let st = array_st(&mut fx.db, fx.bv, &[(0, To, 3)]);
    let e = agg(&mut fx, &mut m, "(0 to 1 => \"01\", others => '1')", st).unwrap();
    assert_eq!(bits(&fx, e).1, "0111");
    assert!(agg(&mut fx, &mut m, "(0 to 2 => \"01\", others => '1')", st).is_err());
    assert_eq!(fx.codes(), [diag::LENGTH_MISMATCH]);
}

#[test]
fn element_of_neither_type_is_e1323() {
    let mut fx = Fx::new();
    let mut m = Mock::new(&fx);
    m.objs.insert(
        Symbol::intern("i"),
        (fx.int_st, Staticness::Locally, Some(ConstValue::Int(1))),
    );
    let bv = fx.bv_st;
    assert!(agg(&mut fx, &mut m, "('1', i)", bv).is_err());
    assert_eq!(fx.codes(), [diag::AGG_ELEMENT]);
}

#[test]
fn array_of_arrays_element_type_wins_tie() {
    // type bva is array (natural range <>) of bit_vector(0 to 1)
    let mut fx = Fx::new();
    let mut m = Mock::new(&fx);
    let e2 = array_st(&mut fx.db, fx.bv, &[(0, To, 1)]);
    let (_, bva) = add_type(
        &mut fx.db,
        "bva",
        TypeKind::Array {
            indexes: Box::new([fx.natural]),
            elem: e2,
        },
        false,
    );
    let e = agg(&mut fx, &mut m, "((others => '1'), \"01\")", bva).unwrap();
    let ExprKind::Aggregate { assocs, .. } = &fx.db.exprs[e].kind else {
        panic!()
    };
    assert!(assocs.iter().all(|a| !a.slice_piece));
    assert!(fx.codes().is_empty());
    let Some(ConstValue::Array(a)) = fx.db.value(e) else {
        panic!()
    };
    assert_eq!(&*a.bounds, &[(0, To, 1)]);
    let Elems::Runs(r) = &a.elems else { panic!() };
    assert_eq!(r.len(), 2);
}

#[test]
fn two_dimensional_with_string_rows_and_others() {
    let mut fx = Fx::new();
    let mut m = Mock::new(&fx);
    let (mt, mst) = add_type(
        &mut fx.db,
        "bm",
        TypeKind::Array {
            indexes: Box::new([fx.natural, fx.natural]),
            elem: fx.bit_st,
        },
        false,
    );
    let e = agg(&mut fx, &mut m, "(\"01\", \"10\", ('1', '1'))", mst).unwrap();
    let Some(ConstValue::Array(a)) = fx.db.value(e) else {
        panic!()
    };
    assert_eq!(&*a.bounds, &[(0, To, 2), (0, To, 1)]);
    let flat: Vec<_> = elem_runs(a)
        .iter()
        .flat_map(|(n, v)| std::iter::repeat_n(v.as_int().unwrap(), *n as usize))
        .collect();
    assert_eq!(flat, [0, 1, 1, 0, 1, 1]);
    // A 1K x 1K memory of zeros is one run.
    let big = array_st(&mut fx.db, mt, &[(0, To, 1023), (0, To, 1023)]);
    let e = agg(&mut fx, &mut m, "(others => (others => '0'))", big).unwrap();
    let Some(ConstValue::Array(a)) = fx.db.value(e) else {
        panic!()
    };
    assert_eq!(elem_runs(a), vec![(1 << 20, ConstValue::Enum(0))]);
    // Rows of different lengths.
    assert!(agg(&mut fx, &mut m, "(\"01\", \"101\")", mst).is_err());
    // A non-aggregate row, reported once.
    assert!(agg(&mut fx, &mut m, "('0', '1')", mst).is_err());
    assert_eq!(fx.codes(), [diag::LENGTH_MISMATCH, diag::AGG_ELEMENT]);
}

#[test]
fn too_many_for_index_subtype_is_e1403() {
    let mut fx = Fx::new();
    let mut m = Mock::new(&fx);
    let small = range_st(&mut fx.db, fx.int_st, 0, To, 1);
    let (_, t) = add_type(
        &mut fx.db,
        "t2",
        TypeKind::Array {
            indexes: Box::new([small]),
            elem: fx.bit_st,
        },
        false,
    );
    assert!(agg(&mut fx, &mut m, "('0', '1', '0')", t).is_err());
    assert_eq!(fx.codes(), [diag::OUT_OF_SUBTYPE]);
}

#[test]
fn subtype_choice_and_enum_index() {
    let mut fx = Fx::new();
    let mut m = Mock::new(&fx);
    let (st_ty, st_st) = add_enum(
        &mut fx.db,
        "state",
        &[
            Designator::Ident(ident("idle")),
            Designator::Ident(ident("run")),
            Designator::Ident(ident("done")),
        ],
    );
    let first2 = range_st(&mut fx.db, st_st, 0, To, 1);
    m.marks.insert(Symbol::intern("first2"), first2);
    let (_, t) = add_type(
        &mut fx.db,
        "per_state",
        TypeKind::Array {
            indexes: Box::new([st_st]),
            elem: fx.bit_st,
        },
        false,
    );
    let _ = st_ty;
    // `first2 => '1'` covers idle and run; the gap at done is reported by name.
    let e = agg(&mut fx, &mut m, "(first2 => '1')", t).unwrap();
    assert_eq!(bits(&fx, e), (vec![(0, To, 1)], "11".into()));
}

// -- record aggregates --------------------------------------------------------

fn rec(fx: &mut Fx) -> SubtypeId {
    let elems = vec![
        RecordElem {
            name: ident("a"),
            subtype: fx.int_st,
        },
        RecordElem {
            name: ident("b"),
            subtype: fx.bit_st,
        },
        RecordElem {
            name: ident("c"),
            subtype: fx.bit_st,
        },
    ];
    add_type(
        &mut fx.db,
        "rec",
        TypeKind::Record {
            elems: elems.into(),
        },
        false,
    )
    .1
}

#[test]
fn record_positional_named_others() {
    let mut fx = Fx::new();
    let mut m = Mock::new(&fx);
    let r = rec(&mut fx);
    let e = agg(&mut fx, &mut m, "(1, c => '1', others => '0')", r).unwrap();
    assert_eq!(
        fx.db.value(e),
        Some(&ConstValue::Record(Box::new([
            ConstValue::Int(1),
            ConstValue::Enum(0),
            ConstValue::Enum(1)
        ])))
    );
    let e = agg(&mut fx, &mut m, "(b | c => '1', a => 7)", r).unwrap();
    let ExprKind::Aggregate { assocs, bounds } = &fx.db.exprs[e].kind else {
        panic!()
    };
    assert!(matches!(bounds, AggBounds::Record));
    assert!(matches!(
        &*assocs[0].choices,
        [Choice::Element(1), Choice::Element(2)]
    ));
    assert!(fx.codes().is_empty());
}

#[test]
fn record_errors() {
    let mut fx = Fx::new();
    let mut m = Mock::new(&fx);
    let r = rec(&mut fx);
    let cases: &[(&str, &str)] = &[
        ("(1, '0')", diag::AGG_MISSING),
        ("(a => 1, a => 2, b => '0', c => '0')", diag::AGG_DUPLICATE),
        ("(a => 1, bb => '0', c => '0')", diag::NOT_DECLARED),
        ("(1, '0', '1', '1')", diag::AGG_DUPLICATE),
        ("(a => 1, '0', '1')", diag::AGG_NONSTATIC_CHOICE),
        (
            "(a => 1, b => '0', c => '0', others => '1')",
            diag::AGG_OTHERS,
        ),
        ("(others => '1')", diag::AGG_ELEMENT),
        ("(a | b => '1', c => '0')", diag::AGG_ELEMENT),
    ];
    for (src, code) in cases {
        let before = fx.db.diagnostics().len();
        assert!(agg(&mut fx, &mut m, src, r).is_err(), "{src}");
        assert_eq!(fx.db.diagnostics()[before].code, *code, "{src}");
    }
    assert!(
        fx.db.diagnostics()[2]
            .help
            .iter()
            .any(|h| h.contains("`b`"))
    );
}

// -- case choices -------------------------------------------------------------

fn case(
    fx: &mut Fx,
    m: &mut Mock,
    src: &str,
    st: SubtypeId,
    matching: bool,
) -> Result<Vec<ChoiceSet>, Reported> {
    let e = parse_expr(fx, src);
    let span = e.span;
    let ty = fx.db.base(st);
    let mut alts = Vec::new();
    for a in assocs(e) {
        alts.push(resolve_choices(&mut fx.db, m, &a.choices, ty)?.into_boxed_slice());
    }
    check_case(&mut fx.db, st, matching, alts, span)
}

#[test]
fn case_enum_missing_and_overlap() {
    let mut fx = Fx::new();
    let mut m = Mock::new(&fx);
    let (_, e4) = add_enum(&mut fx.db, "e4", &chars("abcd"));
    m.enums.push(fx.db.base(e4));
    assert!(case(&mut fx, &mut m, "('a' => 0, 'b' | 'c' => 0)", e4, false).is_err());
    assert!(fx.db.diagnostics()[0].message.contains("'d'"));
    let i8 = range_st(&mut fx.db, fx.int_st, 0, To, 7);
    assert!(case(&mut fx, &mut m, "(0 to 3 => 0, 3 to 6 => 0)", i8, false).is_err());
    assert!(case(&mut fx, &mut m, "(0 to 7 => 0, 8 => 0)", i8, false).is_err());
    assert!(fx.codes().iter().all(|&c| c == diag::CASE_COVERAGE));
    assert_eq!(fx.codes().len(), 4);
    let sets = case(
        &mut fx,
        &mut m,
        "(0 to 3 | 5 => 0, 4 | 6 => 0, others => 0)",
        i8,
        false,
    )
    .unwrap();
    assert_eq!(sets[0].intervals.as_deref(), Some(&[(0, 3), (5, 5)][..]));
    assert_eq!(sets[1].intervals.as_deref(), Some(&[(4, 4), (6, 6)][..]));
}

#[test]
fn case_non_static_subtype_needs_base_coverage() {
    let mut fx = Fx::new();
    let mut m = Mock::new(&fx);
    // INTEGER: covering 0 to 7 is not enough.
    let st = fx.int_st;
    assert!(case(&mut fx, &mut m, "(0 to 7 => 0)", st, false).is_err());
    assert!(case(&mut fx, &mut m, "(0 to 7 => 0, others => 0)", st, false).is_ok());
}

#[test]
fn case_non_static_choice_is_e1410_deferred_is_ok() {
    let mut fx = Fx::new();
    let mut m = Mock::new(&fx);
    m.objs
        .insert(Symbol::intern("g"), (fx.int_st, Staticness::Globally, None));
    m.objs
        .insert(Symbol::intern("f"), (fx.int_st, Staticness::Locally, None));
    let st = fx.int_st;
    assert!(case(&mut fx, &mut m, "(g => 0, others => 0)", st, false).is_err());
    assert_eq!(fx.codes(), [diag::NOT_STATIC]);
    let sets = case(&mut fx, &mut m, "(f => 0, others => 0)", st, false).unwrap();
    assert!(sets[0].intervals.is_none());
    // Elab rechecks with its evaluator.
    struct Eval;
    impl ConstEval for Eval {
        fn eval(&mut self, db: &Db, e: ExprId) -> Option<ConstValue> {
            db.value(e).cloned().or(Some(ConstValue::Int(3)))
        }
    }
    let refs: Vec<&ChoiceSet> = sets.iter().collect();
    recheck_case(&mut fx.db, &mut Eval, st, false, &refs, Span::default()).unwrap();
}

#[test]
fn case_others_must_be_last() {
    let mut fx = Fx::new();
    let mut m = Mock::new(&fx);
    let st = fx.int_st;
    assert!(case(&mut fx, &mut m, "(others => 0, 1 => 0)", st, false).is_err());
    assert_eq!(fx.codes(), [diag::CASE_COVERAGE]);
}

#[test]
fn matching_case_std_ulogic_dont_care() {
    let mut fx = Fx::new();
    let mut m = Mock::new(&fx);
    let st = fx.sul_st;
    // '-' matches everything (as sim1); '1' and 'H' are different values.
    assert!(case(&mut fx, &mut m, "('1' => 0, '0' => 0, '-' => 0)", st, true).is_ok());
    assert!(
        case(
            &mut fx,
            &mut m,
            "('1' => 0, 'H' => 0, others => 0)",
            st,
            true
        )
        .is_ok()
    );
    assert!(case(&mut fx, &mut m, "('1' => 0, '0' => 0)", st, true).is_err());
    assert!(
        case(
            &mut fx,
            &mut m,
            "('1' => 0, '1' => 0, others => 0)",
            st,
            true
        )
        .is_err()
    );
    // case? on INTEGER is not allowed.
    let i = fx.int_st;
    assert!(case(&mut fx, &mut m, "(1 => 0, others => 0)", i, true).is_err());
    assert_eq!(fx.codes(), [diag::CASE_COVERAGE; 3]);
}

#[test]
fn case_on_arrays() {
    let mut fx = Fx::new();
    let mut m = Mock::new(&fx);
    let bv2 = array_st(&mut fx.db, fx.bv, &[(0, To, 1)]);
    assert!(
        case(
            &mut fx,
            &mut m,
            "(\"00\" => 0, \"01\" => 0, \"10\" => 0, \"11\" => 0)",
            bv2,
            false
        )
        .is_ok()
    );
    assert!(
        case(
            &mut fx,
            &mut m,
            "(\"00\" => 0, \"01\" => 0, \"10\" => 0)",
            bv2,
            false
        )
        .is_err()
    );
    assert!(
        case(
            &mut fx,
            &mut m,
            "(\"00\" => 0, \"00\" => 0, others => 0)",
            bv2,
            false
        )
        .is_err()
    );
    assert!(case(&mut fx, &mut m, "(\"000\" => 0, others => 0)", bv2, false).is_err());
    // Unconstrained expression: choices must agree with each other.
    let bv = fx.bv_st;
    assert!(
        case(
            &mut fx,
            &mut m,
            "(\"00\" => 0, \"010\" => 0, others => 0)",
            bv,
            false
        )
        .is_err()
    );
    assert!(
        case(
            &mut fx,
            &mut m,
            "(\"00\" => 0, \"01\" => 0, others => 0)",
            bv,
            false
        )
        .is_ok()
    );
    assert_eq!(
        fx.codes(),
        [
            diag::CASE_COVERAGE,
            diag::CASE_COVERAGE,
            diag::LENGTH_MISMATCH,
            diag::LENGTH_MISMATCH
        ]
    );
}

#[test]
fn matching_case_on_std_ulogic_vector() {
    let mut fx = Fx::new();
    let mut m = Mock::new(&fx);
    let (sulv, _) = add_type(
        &mut fx.db,
        "std_ulogic_vector",
        TypeKind::Array {
            indexes: Box::new([fx.natural]),
            elem: fx.sul_st,
        },
        false,
    );
    let v2 = array_st(&mut fx.db, sulv, &[(1, Downto, 0)]);
    assert!(
        case(
            &mut fx,
            &mut m,
            "(\"1-\" => 0, \"0-\" => 0, \"--\" => 0)",
            v2,
            true
        )
        .is_ok()
    );
    assert!(case(&mut fx, &mut m, "(\"1-\" => 0, \"0-\" => 0)", v2, true).is_err());
}

#[test]
fn static_range_helpers() {
    let r = StaticRange::new(7, Downto, 0);
    assert_eq!((r.low(), r.high(), r.len()), (0, 7, 8));
    assert_eq!(r.offset(7), 0);
    assert!(StaticRange::new(1, To, 0).is_null());
    assert_eq!(StaticRange::new(i64::MIN, To, i64::MAX).len(), 1u128 << 64);
    let fx = Fx::new();
    assert_eq!(
        case_domain(&fx.db, fx.sul_st),
        Some(StaticRange::new(0, To, 8))
    );
    assert_eq!(
        case_domain(&fx.db, fx.int_st).map(|r| r.high()),
        Some(i64::from(i32::MAX))
    );
    let _ = (fx.uint, fx.int, fx.bit, fx.sul);
}
