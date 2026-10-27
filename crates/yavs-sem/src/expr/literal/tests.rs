use super::*;
use crate::decl::{Decl, DeclKind};
use crate::ids::Id;
use crate::types::{PhysUnit, SubtypeDef, TypeDef};
use crate::{LangRev, Options};
use yavs_diag::FileId;

fn sp() -> Span {
    Span::new(FileId(0), 10, 40)
}

pub(crate) fn ident(s: &str) -> Ident {
    let k = Symbol::intern(&s.to_ascii_lowercase());
    Ident {
        text: Symbol::intern(s),
        key: k,
        span: Span::new(FileId(0), 20, 22),
    }
}

pub(crate) fn new_db() -> Db {
    Db::new(Options::new(LangRev::Vhdl2008, "/nonexistent"))
}

pub(crate) fn add_type(db: &mut Db, name: &str, kind: TypeKind) -> TypeId {
    let root = db.root;
    let decl = db.decls.push(Decl {
        name: Designator::Ident(ident(name)),
        kind: DeclKind::Error,
        scope: root,
        span: Span::default(),
        implicit: false,
        since: LangRev::Vhdl1993,
    });
    let ty = db.types.next_id();
    let st = db.subtypes.push(SubtypeDef {
        base: ty,
        name: Some(ident(name)),
        parent: None,
        constraint: Constraint::None,
        resolution: None,
        span: Span::default(),
    });
    db.decls[decl].kind = DeclKind::Type(ty);
    db.types.push(TypeDef {
        decl,
        name: ident(name),
        anonymous: false,
        kind,
        first_subtype: st,
        base_subtype: st,
    });
    ty
}

fn int_expr(db: &mut Db, v: i64) -> ExprId {
    let c = db.add_const(ConstValue::Int(v));
    let st = db.poison.subtype;
    db.add_expr(Expr {
        kind: ExprKind::Const(c),
        subtype: st,
        span: Span::default(),
        stat: Staticness::Locally,
        value: Some(c),
    })
}

/// Enumeration type of character literals.
pub(crate) fn char_enum(db: &mut Db, name: &str, chars: &str) -> TypeId {
    let root = db.root;
    let ty = add_type(db, name, TypeKind::Enum { lits: Box::new([]) });
    let lits: Vec<_> = chars
        .chars()
        .enumerate()
        .map(|(pos, c)| {
            db.decls.push(Decl {
                name: Designator::Char(c),
                kind: DeclKind::EnumLit {
                    ty,
                    pos: pos as u32,
                },
                scope: root,
                span: Span::default(),
                implicit: true,
                since: LangRev::Vhdl1993,
            })
        })
        .collect();
    db.types[ty].kind = TypeKind::Enum {
        lits: lits.into_boxed_slice(),
    };
    ty
}

/// `array (<index> range <>) of <elem>` with index subtype `lo to hi` (or `lo downto hi`).
pub(crate) fn array_of(
    db: &mut Db,
    name: &str,
    elem: TypeId,
    lo: i64,
    dir: Direction,
    hi: i64,
) -> TypeId {
    let int = add_type(db, "idx", TypeKind::Integer);
    let l = int_expr(db, lo);
    let r = int_expr(db, hi);
    let idx = db.subtypes.push(SubtypeDef {
        base: int,
        name: None,
        parent: None,
        constraint: Constraint::Range(RangeC::Explicit {
            left: l,
            dir,
            right: r,
        }),
        resolution: None,
        span: Span::default(),
    });
    let es = db.types[elem].first_subtype;
    add_type(
        db,
        name,
        TypeKind::Array {
            indexes: Box::new([idx]),
            elem: es,
        },
    )
}

const STD_ULOGIC: &str = "UX01ZWLH-";

struct Env {
    db: Db,
    slv: TypeId,
    string: TypeId,
    bit_vector: TypeId,
}

fn env() -> Env {
    let mut db = new_db();
    let ul = char_enum(&mut db, "std_ulogic", STD_ULOGIC);
    let slv = array_of(
        &mut db,
        "std_logic_vector",
        ul,
        0,
        Direction::To,
        i32::MAX.into(),
    );
    let bit = char_enum(&mut db, "bit", "01");
    let bit_vector = array_of(
        &mut db,
        "bit_vector",
        bit,
        0,
        Direction::To,
        i32::MAX.into(),
    );
    // Every Latin-1 value has a character literal here (std only has the graphic ones).
    let latin1: String = (0u8..=255).map(char::from).collect();
    let ch = char_enum(&mut db, "character", &latin1);
    let string = array_of(&mut db, "string", ch, 1, Direction::To, i32::MAX.into());
    Env {
        db,
        slv,
        string,
        bit_vector,
    }
}

fn expand(t: &str) -> String {
    String::from_utf8(expand_bitstring(t).unwrap_or_else(|e| panic!("{t}: {e:?}"))).unwrap()
}

fn lost(t: &str) -> bool {
    matches!(expand_bitstring(t), Err(BitStringError::Lost { .. }))
}

// -- abstract literals --------------------------------------------------------

#[test]
fn integers() {
    let int = |t| abstract_literal(t);
    assert_eq!(int("0"), Ok(Abs::Int(0)));
    assert_eq!(int("1_000_000"), Ok(Abs::Int(1_000_000)));
    assert_eq!(int("1e3"), Ok(Abs::Int(1000)));
    assert_eq!(int("1E+3"), Ok(Abs::Int(1000)));
    assert_eq!(int("0e99"), Ok(Abs::Int(0)));
    assert_eq!(int("16#FF#"), Ok(Abs::Int(255)));
    assert_eq!(int("16#ff#"), Ok(Abs::Int(255)));
    assert_eq!(int("16:FF:"), Ok(Abs::Int(255)));
    assert_eq!(int("2#1010_1010#"), Ok(Abs::Int(170)));
    assert_eq!(int("2#1#E3"), Ok(Abs::Int(8)));
    assert_eq!(int("8#777#"), Ok(Abs::Int(511)));
    assert_eq!(int("3#2#"), Ok(Abs::Int(2)));
    assert_eq!(int("9223372036854775807"), Ok(Abs::Int(i64::MAX)));
    assert_eq!(int("16#7FFF_FFFF_FFFF_FFFF#"), Ok(Abs::Int(i64::MAX)));
}

#[test]
fn integer_errors() {
    assert_eq!(
        abstract_literal("9223372036854775808"),
        Err(AbsError::TooLarge("an integer literal"))
    );
    assert_eq!(
        abstract_literal("16#FFFF_FFFF_FFFF_FFFF#"),
        Err(AbsError::TooLarge("an integer literal"))
    );
    assert_eq!(
        abstract_literal("1e30"),
        Err(AbsError::TooLarge("an integer literal"))
    );
    assert_eq!(
        abstract_literal("340282366920938463463374607431768211456"),
        Err(AbsError::TooLarge("an integer literal"))
    );
    assert_eq!(
        abstract_literal("1e99999999999999999999"),
        Err(AbsError::TooLarge("the exponent"))
    );
    for bad in [
        "1e-3", "16#G#", "1#0#", "17#1#", "2#2#", "16#FF", "1__0", "1_", "x", "", "1 2",
    ] {
        assert!(
            matches!(abstract_literal(bad), Err(AbsError::Malformed(_))),
            "{bad}: {:?}",
            abstract_literal(bad)
        );
    }
}

/// Based reals are correctly rounded, also below the normal range (#105; sim1 flushes this one).
#[test]
fn based_reals_are_correctly_rounded() {
    assert_eq!(abstract_literal("3#0.1#"), Ok(Abs::Real(1.0 / 3.0)));
    assert_eq!(
        abstract_literal("2#1.1#E-1075"),
        Ok(Abs::Real(f64::from_bits(1)))
    );
    assert_eq!(
        abstract_literal("16#20000000000001.00000000001#"),
        Ok(Abs::Real(9007199254740994.0))
    );
}

#[test]
fn reals() {
    let real = |t| abstract_literal(t);
    assert_eq!(real("1.5"), Ok(Abs::Real(1.5)));
    assert_eq!(real("1_0.0_1"), Ok(Abs::Real(10.01)));
    assert_eq!(real("1.0e-3"), Ok(Abs::Real(1e-3)));
    assert_eq!(real("1.0E+3"), Ok(Abs::Real(1000.0)));
    assert_eq!(real("0.1"), Ok(Abs::Real(0.1)));
    assert_eq!(real("2#1.1#"), Ok(Abs::Real(1.5)));
    assert_eq!(real("2#1.1#E3"), Ok(Abs::Real(12.0)));
    assert_eq!(real("2#1.1#E-1"), Ok(Abs::Real(0.75)));
    assert_eq!(real("16#F.8#"), Ok(Abs::Real(15.5)));
    assert_eq!(real("16#0.8#E1"), Ok(Abs::Real(8.0)));
    assert_eq!(real("16:A.0:"), Ok(Abs::Real(10.0)));
    assert_eq!(real("10#1.5#"), Ok(Abs::Real(1.5)));
    assert_eq!(real("3#0.1#"), Ok(Abs::Real(1.0 / 3.0)));
    assert_eq!(real("5#1.2#"), Ok(Abs::Real(1.4)));
    assert_eq!(real("1.7976931348623157e308"), Ok(Abs::Real(f64::MAX)));
    // Subnormals and underflow are values, not errors.
    assert_eq!(real("4.9e-324"), Ok(Abs::Real(5e-324)));
    assert_eq!(real("1.0e-400"), Ok(Abs::Real(0.0)));
    // Nearest, ties to even: 2^53 + 1 is a tie between 2^53 and 2^53 + 2.
    assert_eq!(
        real("9007199254740993.0"),
        Ok(Abs::Real(9007199254740992.0))
    );
    assert_eq!(
        real("9007199254740995.0"),
        Ok(Abs::Real(9007199254740996.0))
    );
    assert_eq!(
        real("16#20_0000_0000_0001.0#"),
        Ok(Abs::Real(9007199254740992.0))
    );
    assert_eq!(
        real("16#20_0000_0000_0003.0#"),
        Ok(Abs::Real(9007199254740996.0))
    );
}

#[test]
fn real_errors() {
    assert_eq!(
        abstract_literal("1.0e309"),
        Err(AbsError::TooLarge("a real literal"))
    );
    assert_eq!(
        abstract_literal("16#F.F#E300"),
        Err(AbsError::TooLarge("a real literal"))
    );
    assert_eq!(
        abstract_literal("2#1.0#E99999999"),
        Err(AbsError::TooLarge("a real literal"))
    );
    for bad in ["1.", "1.e3", ".5", "1.0e", "2#1.2#", "16#F.#", "1.0.0"] {
        assert!(
            matches!(abstract_literal(bad), Err(AbsError::Malformed(_))),
            "{bad}: {:?}",
            abstract_literal(bad)
        );
    }
}

// -- physical -----------------------------------------------------------------

#[test]
fn physical() {
    assert_eq!(physical_literal("10", 1_000_000), Ok(10_000_000));
    assert_eq!(physical_literal("1.5", 1_000), Ok(1_500));
    // Rounded to the primary unit, ties away from zero.
    assert_eq!(physical_literal("1.4", 1), Ok(1));
    assert_eq!(physical_literal("1.5", 1), Ok(2));
    assert_eq!(physical_literal("2.5", 1), Ok(3));
    assert_eq!(physical_literal("0.4999", 1), Ok(0));
    assert_eq!(physical_literal("1e3", 1_000), Ok(1_000_000));
    assert_eq!(physical_literal("1.0e-3", 1_000_000), Ok(1_000));
    assert_eq!(physical_literal("16#10#", 1_000), Ok(16_000));
    assert_eq!(physical_literal("0", i64::MAX), Ok(0));
    assert_eq!(physical_literal("9223372036854775807", 1), Ok(i64::MAX));
    assert_eq!(
        physical_literal("9223372036854775808", 1),
        Err(AbsError::TooLarge("a physical literal"))
    );
    assert_eq!(
        physical_literal("9223372036854", 1_000_000_000),
        Err(AbsError::TooLarge("a physical literal"))
    );
    assert_eq!(physical_literal("1.0e-30", 1_000), Ok(0));
    assert_eq!(physical_literal("1e-1", 5), Ok(1));
}

#[test]
fn physical_typed() {
    let mut db = new_db();
    let time = add_type(
        &mut db,
        "time",
        TypeKind::Physical {
            units: Box::new([]),
        },
    );
    let root = db.root;
    let mut units = Vec::new();
    for (i, (n, scale)) in [("fs", 1i64), ("ps", 1000), ("ns", 1_000_000)]
        .iter()
        .enumerate()
    {
        let decl = db.decls.push(Decl {
            name: Designator::Ident(ident(n)),
            kind: DeclKind::PhysUnit {
                ty: time,
                index: i as u32,
            },
            scope: root,
            span: Span::default(),
            implicit: false,
            since: LangRev::Vhdl1993,
        });
        units.push(PhysUnit {
            decl,
            scale: *scale,
        });
    }
    db.types[time].kind = TypeKind::Physical {
        units: units.into_boxed_slice(),
    };
    let v = physical_value(&mut db, Symbol::intern("10"), ident("NS"), time, sp());
    assert_eq!(v, Ok(ConstValue::Phys(10_000_000)));
    let v = physical_value(&mut db, Symbol::intern("1.5"), ident("ps"), time, sp());
    assert_eq!(v, Ok(ConstValue::Phys(1500)));
    assert_eq!(db.error_count(), 0);
    let v = physical_value(&mut db, Symbol::intern("1"), ident("sec"), time, sp());
    assert_eq!(v, Err(Reported));
    assert_eq!(
        db.diagnostics().last().map(|d| d.code),
        Some(diag::BAD_LITERAL)
    );
    let v = physical_value(
        &mut db,
        Symbol::intern("9223372036854775807"),
        ident("ns"),
        time,
        sp(),
    );
    assert_eq!(v, Err(Reported));
    assert_eq!(
        db.diagnostics().last().map(|d| d.code),
        Some(diag::LITERAL_RANGE)
    );
    // Not a physical type.
    let e = env();
    let mut db = e.db;
    let r = physical_value(&mut db, Symbol::intern("1"), ident("ns"), e.slv, sp());
    assert_eq!(r, Err(Reported));
    assert_eq!(
        db.diagnostics().last().map(|d| d.code),
        Some(diag::TYPE_MISMATCH)
    );
}

// -- bit strings --------------------------------------------------------------

#[test]
fn bits_basic() {
    assert_eq!(expand("B\"101\""), "101");
    assert_eq!(expand("b\"1_0\""), "10");
    assert_eq!(expand("O\"75\""), "111101");
    assert_eq!(expand("X\"A5\""), "10100101");
    assert_eq!(expand("x\"a5\""), "10100101");
    assert_eq!(expand("X\"f_F\""), "11111111");
    assert_eq!(expand("B\"\""), "");
    assert_eq!(expand("X\"\""), "");
    assert_eq!(expand("UB\"1\""), "1");
    assert_eq!(expand("SB\"1\""), "1");
    assert_eq!(expand("ux\"F\""), "1111");
    assert_eq!(expand("sO\"7\""), "111");
}

#[test]
fn bits_percent_delimiter() {
    assert_eq!(expand("B%101%"), "101");
    assert_eq!(expand("8x%F%"), "00001111");
}

#[test]
fn bits_nonbinary_replicated() {
    assert_eq!(expand("X\"1X\""), "0001XXXX");
    assert_eq!(expand("X\"U-Z\""), "UUUU----ZZZZ");
    assert_eq!(expand("O\"W7\""), "WWW111");
    assert_eq!(expand("B\"1XZ\""), "1XZ");
    // Not a digit of the base: replicated, whatever the character.
    assert_eq!(expand("B\"2\""), "2");
    assert_eq!(expand("O\"78\""), "111888");
    assert_eq!(expand("X\"G\""), "GGGG");
    assert_eq!(expand("X\"x\""), "xxxx");
    assert_eq!(expand("8SX\"G\""), "GGGGGGGG");
}

#[test]
fn bits_decimal() {
    assert_eq!(expand("D\"255\""), "11111111");
    assert_eq!(expand("D\"256\""), "100000000");
    assert_eq!(expand("D\"12\""), "1100");
    assert_eq!(expand("D\"1_0\""), "1010");
    assert_eq!(expand("D\"0\""), "");
    assert_eq!(expand("D\"0010\""), "1010");
    assert_eq!(expand("5D\"3\""), "00011");
    assert_eq!(expand("5D\"0\""), "00000");
    assert_eq!(expand("2D\"3\""), "11");
    assert_eq!(expand("3D\"0007\""), "111");
    assert_eq!(
        expand("D\"18446744073709551616\""),
        format!("1{}", "0".repeat(64))
    );
    assert!(lost("2D\"4\""));
    assert!(matches!(
        expand_bitstring("D\"A\""),
        Err(BitStringError::BadDecimalDigit { ch: 'A', .. })
    ));
    assert!(matches!(
        expand_bitstring("D\"1X\""),
        Err(BitStringError::BadDecimalDigit { offset: 3, ch: 'X' })
    ));
}

#[test]
fn bits_length_extend() {
    assert_eq!(expand("8X\"F\""), "00001111");
    assert_eq!(expand("8UX\"F\""), "00001111");
    assert_eq!(expand("8SX\"F\""), "11111111");
    assert_eq!(expand("8SX\"1\""), "00000001");
    assert_eq!(expand("8SX\"X\""), "XXXXXXXX");
    assert_eq!(expand("8UX\"U\""), "0000UUUU");
    assert_eq!(expand("12SO\"7\""), "111111111111");
    assert_eq!(expand("8UX\"\""), "00000000");
    assert_eq!(expand("8B\"\""), "00000000");
    assert_eq!(expand("0X\"\""), "");
    assert_eq!(expand("SX\"\""), "");
    assert_eq!(expand("0SX\"\""), "");
    assert_eq!(expand("4SX\"F\""), "1111");
    assert_eq!(expand("6_4B\"1\"").len(), 64);
}

#[test]
fn bits_length_truncate_unsigned() {
    assert_eq!(expand("3X\"3\""), "011");
    assert_eq!(expand("4UB\"0XXXX\""), "XXXX");
    assert_eq!(expand("0UB\"0\""), "");
    assert_eq!(expand("0B\"000\""), "");
    assert_eq!(expand("2UB\"1_0\""), "10");
    assert_eq!(expand("8X\"0F\""), "00001111");
    assert_eq!(expand("4X\"00F\""), "1111");
    assert!(lost("3X\"FF\""));
    assert!(lost("3UX\"1F\""));
    assert!(lost("0UB\"1\""));
    assert!(lost("4UB\"XXXXX\""));
    assert!(lost("3UX\"X\""));
    assert!(lost("1B\"10\""));
    match expand_bitstring("3X\"FF\"") {
        Err(BitStringError::Lost {
            dropped,
            expected,
            signed,
        }) => assert_eq!(
            (dropped, expected, signed),
            (b"11111".to_vec(), b'0', false)
        ),
        other => panic!("{other:?}"),
    }
}

#[test]
fn bits_length_truncate_signed() {
    // Dropped characters must equal the leftmost kept one (LRM; sim2; sim1 is laxer).
    assert_eq!(expand("3SX\"F\""), "111");
    assert_eq!(expand("2SB\"001\""), "01");
    assert_eq!(expand("1SB\"11\""), "1");
    assert_eq!(expand("1SB\"00\""), "0");
    assert_eq!(expand("2SX\"UU\""), "UU");
    assert_eq!(expand("0SB\"\""), "");
    assert!(lost("4SX\"7F\""));
    assert!(lost("1SB\"10\""));
    assert!(lost("2SB\"101\""));
    assert!(lost("2SX\"XU\""));
    assert_eq!(
        expand_bitstring("8SX\"\""),
        Err(BitStringError::EmptySigned)
    );
    match expand_bitstring("2SB\"101\"") {
        Err(BitStringError::Lost {
            dropped,
            expected,
            signed,
        }) => assert_eq!((dropped, expected, signed), (b"1".to_vec(), b'0', true)),
        other => panic!("{other:?}"),
    }
}

#[test]
fn bits_errors() {
    assert!(matches!(
        expand_bitstring("Q\"1\""),
        Err(BitStringError::Malformed(_))
    ));
    assert!(matches!(
        expand_bitstring("SD\"1\""),
        Err(BitStringError::Malformed(_))
    ));
    assert!(matches!(
        expand_bitstring("X1"),
        Err(BitStringError::Malformed(_))
    ));
    assert!(matches!(
        expand_bitstring("X\"1"),
        Err(BitStringError::Malformed(_))
    ));
    assert!(matches!(
        expand_bitstring("99999999999999999999999X\"1\""),
        Err(BitStringError::TooLong)
    ));
    assert!(matches!(
        expand_bitstring("9999999999X\"1\""),
        Err(BitStringError::TooLong)
    ));
    assert!(matches!(
        expand_bitstring("X\"\u{3b1}\""),
        Err(BitStringError::NotLatin1 {
            offset: 2,
            ch: '\u{3b1}'
        })
    ));
    // Latin-1 characters above ASCII are fine.
    assert_eq!(expand_bitstring("B\"\u{e9}\"").unwrap(), vec![0xE9]);
}

// -- typed --------------------------------------------------------------------

fn elems(v: &ConstValue) -> Vec<u8> {
    match v {
        ConstValue::Array(ArrayConst {
            elems: Elems::Bytes(b),
            ..
        }) => b.to_vec(),
        other => panic!("{other:?}"),
    }
}

fn bounds(v: &ConstValue) -> (i64, Direction, i64) {
    match v {
        ConstValue::Array(a) => a.bounds[0],
        other => panic!("{other:?}"),
    }
}

#[test]
fn typed_bit_strings() {
    let mut e = env();
    let v = bit_string_literal(&mut e.db, Symbol::intern("X\"1X\""), e.slv, sp()).unwrap();
    // std_ulogic positions: U=0 X=1 0=2 1=3 ...
    assert_eq!(elems(&v), vec![2, 2, 2, 3, 1, 1, 1, 1]);
    assert_eq!(bounds(&v), (0, Direction::To, 7));
    let v = bit_string_literal(&mut e.db, Symbol::intern("B\"\""), e.slv, sp()).unwrap();
    assert_eq!(elems(&v), Vec::<u8>::new());
    assert_eq!(bounds(&v), (0, Direction::To, -1));
    let v = bit_string_literal(&mut e.db, Symbol::intern("B\"10\""), e.bit_vector, sp()).unwrap();
    assert_eq!(elems(&v), vec![1, 0]);
    assert_eq!(e.db.error_count(), 0);
}

#[test]
fn typed_bit_string_errors() {
    let mut e = env();
    let cases: &[(&str, TypeId, &str)] = &[
        ("X\"1X\"", e.bit_vector, diag::CHAR_NOT_IN_TYPE),
        ("B\"2\"", e.slv, diag::CHAR_NOT_IN_TYPE),
        ("3X\"FF\"", e.slv, diag::BITSTRING_DIGITS),
        ("8SX\"\"", e.slv, diag::BITSTRING_DIGITS),
        ("D\"A\"", e.slv, diag::BAD_LITERAL),
        ("Q\"1\"", e.slv, diag::BAD_LITERAL),
        ("9999999999X\"1\"", e.slv, diag::LITERAL_RANGE),
        ("X\"\u{3b1}\"", e.slv, diag::CHAR_NOT_IN_TYPE),
    ];
    for (t, ty, code) in cases {
        let r = bit_string_literal(&mut e.db, Symbol::intern(t), *ty, sp());
        assert_eq!(r, Err(Reported), "{t}");
        assert_eq!(
            e.db.diagnostics().last().map(|d| d.code),
            Some(*code),
            "{t}"
        );
    }
}

#[test]
fn typed_strings_and_chars() {
    let mut e = env();
    let v = string_literal(&mut e.db, Symbol::intern("a\"b"), e.string, sp()).unwrap();
    assert_eq!(elems(&v), vec![97, 34, 98]);
    assert_eq!(bounds(&v), (1, Direction::To, 3));
    let v = string_literal(&mut e.db, Symbol::intern(""), e.string, sp()).unwrap();
    assert_eq!(bounds(&v), (1, Direction::To, 0));
    // Latin-1.
    let v = string_literal(&mut e.db, Symbol::intern("\u{e9}\u{ff}"), e.string, sp()).unwrap();
    assert_eq!(elems(&v), vec![0xE9, 0xFF]);
    // Not a character of the type.
    let r = string_literal(&mut e.db, Symbol::intern("01x"), e.slv, sp());
    assert_eq!(r, Err(Reported));
    let d = e.db.diagnostics().last().unwrap();
    assert_eq!(d.code, diag::CHAR_NOT_IN_TYPE);
    assert!(d.message.contains("'x'"), "{}", d.message);
    // Above U+00FF.
    let r = string_literal(&mut e.db, Symbol::intern("a\u{20ac}"), e.string, sp());
    assert_eq!(r, Err(Reported));
    assert_eq!(
        e.db.diagnostics().last().unwrap().code,
        diag::CHAR_NOT_IN_TYPE
    );
    // Characters.
    let ul = e.db.subtypes[match &e.db.types[e.slv].kind {
        TypeKind::Array { elem, .. } => *elem,
        _ => unreachable!(),
    }]
    .base;
    assert_eq!(
        char_literal(&mut e.db, 'Z', ul, sp()),
        Ok(ConstValue::Enum(4))
    );
    assert_eq!(char_literal(&mut e.db, 'q', ul, sp()), Err(Reported));
    assert_eq!(char_literal(&mut e.db, 'Z', e.slv, sp()), Err(Reported));
    assert_eq!(e.db.diagnostics().last().unwrap().code, diag::TYPE_MISMATCH);
    // A string over a non-character element.
    let int = add_type(&mut e.db, "my_int", TypeKind::Integer);
    let arr = array_of(&mut e.db, "ints", int, 0, Direction::To, 9);
    assert_eq!(
        string_literal(&mut e.db, Symbol::intern("1"), arr, sp()),
        Err(Reported)
    );
    assert_eq!(
        e.db.diagnostics().last().unwrap().code,
        diag::CHAR_NOT_IN_TYPE
    );
}

#[test]
fn string_bounds_follow_the_index_subtype() {
    let mut e = env();
    let ul = e.db.subtypes[match &e.db.types[e.slv].kind {
        TypeKind::Array { elem, .. } => *elem,
        _ => unreachable!(),
    }]
    .base;
    let down = array_of(&mut e.db, "down", ul, 7, Direction::Downto, 0);
    let v = string_literal(&mut e.db, Symbol::intern("01"), down, sp()).unwrap();
    assert_eq!(bounds(&v), (7, Direction::Downto, 6));
    let v = string_literal(&mut e.db, Symbol::intern(""), down, sp()).unwrap();
    assert_eq!(bounds(&v), (7, Direction::Downto, 8));
    // Does not fit a small index subtype.
    let small = array_of(&mut e.db, "small", ul, 0, Direction::To, 1);
    assert_eq!(
        string_literal(&mut e.db, Symbol::intern("010"), small, sp()),
        Err(Reported)
    );
    assert_eq!(
        e.db.diagnostics().last().unwrap().code,
        diag::OUT_OF_SUBTYPE
    );
    // Index arithmetic overflow.
    let top = array_of(&mut e.db, "top", ul, i64::MAX - 1, Direction::To, i64::MAX);
    assert!(string_literal(&mut e.db, Symbol::intern("0"), top, sp()).is_ok());
    assert_eq!(
        string_literal(&mut e.db, Symbol::intern("000"), top, sp()),
        Err(Reported)
    );
    assert_eq!(e.db.diagnostics().last().unwrap().code, diag::OVERFLOW);
}

#[test]
fn literal_exprs() {
    use yavs_syntax::ast::{Literal, LiteralKind};
    let mut e = env();
    let root = e.db.root;
    let int = add_type(&mut e.db, "integer", TypeKind::Integer);
    let real = add_type(&mut e.db, "real", TypeKind::Floating);
    let mk = |k| Literal {
        kind: k,
        span: sp(),
    };
    let id = literal_expr(
        &mut e.db,
        root,
        &mk(LiteralKind::Abstract(Symbol::intern("16#10#"))),
        int,
    )
    .unwrap();
    assert_eq!(e.db.value(id), Some(&ConstValue::Int(16)));
    assert_eq!(e.db.exprs[id].stat, Staticness::Locally);
    let id = literal_expr(
        &mut e.db,
        root,
        &mk(LiteralKind::Abstract(Symbol::intern("1.5"))),
        real,
    )
    .unwrap();
    assert_eq!(e.db.value(id), Some(&ConstValue::Real(1.5)));
    // 1 is not a real literal; 1.0 is not an integer literal.
    for (t, ty) in [("1", real), ("1.0", int)] {
        let r = literal_expr(
            &mut e.db,
            root,
            &mk(LiteralKind::Abstract(Symbol::intern(t))),
            ty,
        );
        assert_eq!(r.map(|_| ()), Err(Reported), "{t}");
        assert_eq!(e.db.diagnostics().last().unwrap().code, diag::TYPE_MISMATCH);
    }
    let r = literal_expr(
        &mut e.db,
        root,
        &mk(LiteralKind::Abstract(Symbol::intern(
            "99999999999999999999",
        ))),
        int,
    );
    assert_eq!(r.map(|_| ()), Err(Reported));
    assert_eq!(e.db.diagnostics().last().unwrap().code, diag::LITERAL_RANGE);
    // The characters of the element type must be visible at the literal (LRM 9.3.2).
    let bad = literal_expr(
        &mut e.db,
        root,
        &mk(LiteralKind::BitString(Symbol::intern("x\"F\""))),
        e.slv,
    );
    assert_eq!(bad.map(|_| ()), Err(Reported));
    assert_eq!(e.db.diagnostics().last().unwrap().code, diag::NOT_DECLARED);
    let TypeKind::Array { elem, .. } = &e.db.types[e.slv].kind else {
        panic!("array")
    };
    let TypeKind::Enum { lits } = &e.db.types[e.db.subtypes[*elem].base].kind else {
        panic!("enum")
    };
    let lits = lits.to_vec();
    for l in lits {
        e.db.declare(root, l);
    }
    let id = literal_expr(
        &mut e.db,
        root,
        &mk(LiteralKind::BitString(Symbol::intern("x\"F\""))),
        e.slv,
    )
    .unwrap();
    assert!(matches!(e.db.value(id), Some(ConstValue::Array(_))));
    let id = literal_expr(&mut e.db, root, &mk(LiteralKind::Null), int).unwrap();
    assert!(matches!(e.db.exprs[id].kind, ExprKind::Null));
    // Poison type: silent.
    let n = e.db.error_count();
    let poison = e.db.poison.ty;
    let r = literal_expr(
        &mut e.db,
        root,
        &mk(LiteralKind::Abstract(Symbol::intern("1"))),
        poison,
    );
    assert_eq!(r.map(|_| ()), Err(Reported));
    let _ = poison.index();
    assert_eq!(e.db.error_count(), n);
}

/// Black-box cross-check against sim1 (`cargo test -p yavs-sem -- --ignored sim1_cross_check`).
/// Valid literals are compared in one run; invalid ones must be rejected by sim1 too. sim1 only
/// requires dropped characters of a signed literal to equal each other, so signed truncation
/// errors (which follow the LRM and sim2) are not asserted against it.
#[test]
#[ignore = "needs sim1 (YAVS_SIM1_CMD) and systemd-run"]
fn sim1_cross_check() {
    use std::process::Command;
    let ok = [
        "B\"101\"",
        "X\"1X\"",
        "X\"U-Z\"",
        "X\"a\"",
        "X\"A_b\"",
        "D\"255\"",
        "D\"0\"",
        "D\"1_0\"",
        "8X\"F\"",
        "3X\"3\"",
        "4SX\"F\"",
        "3SX\"F\"",
        "8SX\"F\"",
        "8SX\"1\"",
        "8SX\"X\"",
        "8UX\"U\"",
        "0X\"\"",
        "8UX\"\"",
        "SX\"\"",
        "5D\"3\"",
        "2D\"3\"",
        "0B\"\"",
        "0UB\"0\"",
        "1SB\"11\"",
        "2SB\"001\"",
        "2UB\"1_0\"",
        "12SO\"7\"",
        "4UB\"0XXXX\"",
        "2SX\"UU\"",
        "UB\"1\"",
        "SB\"1\"",
        "O\"75\"",
        "7UO\"7\"",
        "16D\"65535\"",
        "D\"4294967296\"",
    ];
    let bad = [
        "3X\"FF\"",
        "3UX\"1F\"",
        "2D\"4\"",
        "0UB\"1\"",
        "4UB\"XXXXX\"",
        "3UX\"X\"",
        "8SX\"\"",
        "D\"A\"",
        "D\"1X\"",
    ];
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    // The reference command (analyse, elaborate and run t.vhd in the current directory) is not tracked.
    let cmd =
        std::env::var("YAVS_SIM1_CMD").expect("YAVS_SIM1_CMD: command line running t.vhd on sim1");
    let sim1 = |dir: &std::path::Path, src: &str| -> (bool, String) {
        std::fs::write(dir.join("t.vhd"), src).unwrap();
        let o = Command::new("timeout")
            .args([
                "60",
                "systemd-run",
                "--user",
                "--scope",
                "-q",
                "-p",
                "MemoryMax=1G",
            ])
            .args(["bash", "-c", &cmd])
            .current_dir(dir)
            .output()
            .expect("run sim1");
        (
            o.status.success(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            ),
        )
    };
    let wrap = |decls: &str, body: &str| {
        format!(
            "library ieee; use ieee.std_logic_1164.all;\nentity t is end;\narchitecture a of t is\n{decls}begin\nprocess begin\n{body}wait;\nend process;\nend;\n"
        )
    };
    let mut decls = String::new();
    let mut body = String::new();
    let mut want = Vec::new();
    for (i, lit) in ok.iter().enumerate() {
        decls += &format!("constant c{i} : std_logic_vector := {lit};\n");
        body += &format!(
            "report \"#{i} \" & integer'image(c{i}'length) & \" [\" & to_string(c{i}) & \"]\";\n"
        );
        let v = expand(lit);
        want.push(format!("#{i} {} [{v}]", v.len()));
    }
    let (success, out) = sim1(&dir, &wrap(&decls, &body));
    assert!(success, "{out}");
    for w in &want {
        assert!(out.contains(w), "sim1 output lacks `{w}`:\n{out}");
    }
    for lit in bad {
        assert!(expand_bitstring(lit).is_err(), "{lit}");
        let (success, out) = sim1(
            &dir,
            &wrap(&format!("constant c : std_logic_vector := {lit};\n"), ""),
        );
        assert!(!success, "sim1 accepts {lit}: {out}");
    }
}
