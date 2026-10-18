//! Property tests: 'VALUE('IMAGE(x)) == x, and textio WRITE then READ.

use proptest::prelude::*;
use yavs_std::image::*;
use yavs_std::textio::{self, Side};
use yavs_std::types::*;
use yavs_std::value::*;

fn ident() -> impl Strategy<Value = EnumLit> {
    prop_oneof![
        "[A-Za-z]([A-Za-z0-9]|_[A-Za-z0-9]){0,6}".prop_map(|s| EnumLit::ident(&s)),
        "[A-Za-z0-9 !#-\\[\\]-~]{1,6}".prop_map(|s| EnumLit::ident(&format!("\\{s}\\"))),
        r"[a-z]\\\\[a-z]".prop_map(|s| EnumLit::ident(&format!("\\{s}\\"))),
        (32u8..127).prop_map(EnumLit::Char),
    ]
}

/// Literals of one type must be distinct as the LRM requires (basic
/// identifiers case-insensitively).
fn distinct(lits: &[EnumLit]) -> bool {
    let key = |l: &EnumLit| match l {
        EnumLit::Ident(n) if n.first() != Some(&b'\\') => (0, n.to_ascii_lowercase()),
        EnumLit::Ident(n) => (1, n.clone()),
        EnumLit::Char(c) => (2, vec![*c]),
    };
    let mut k: Vec<_> = lits.iter().map(key).collect();
    k.sort();
    k.windows(2).all(|w| w[0] != w[1])
}

proptest! {
    #[test]
    fn integer(x: i64) {
        prop_assert_eq!(integer_value(&integer_image(x), Bounds::I64), Ok(x));
        let mut l = Vec::new();
        textio::write_integer(&mut l, x, Side::Left, 3);
        prop_assert_eq!(textio::read_integer(&mut &l[..], Bounds::I64), Ok(x));
    }

    #[test]
    fn real_17_digits(x in any::<f64>().prop_filter("finite", |x| x.is_finite())) {
        let s = real_to_string_digits(x, 0).unwrap();
        let back = real_value(&s, RealBounds::REAL).unwrap();
        prop_assert_eq!(back.to_bits(), x.to_bits());
        prop_assert_eq!(textio::read_real(&mut &s[..], RealBounds::REAL).unwrap().to_bits(), x.to_bits());
    }

    #[test]
    fn real_image_15_digits(x in any::<f64>().prop_filter("normal", |x| x.is_normal())) {
        let back = real_value(&real_image(x), RealBounds::REAL).unwrap();
        prop_assert!(((back - x) / x).abs() < 1e-14);
    }

    #[test]
    fn time(t: i64, u in 0usize..8) {
        let ty = time_type();
        prop_assert_eq!(physical_value(&physical_image(t, &ty), &ty, Bounds::I64), Ok(t));
        let unit = TIME_UNITS[u].1;
        let exact = t - t % unit;
        let s = physical_to_string_unit(exact, &ty, unit).unwrap();
        prop_assert_eq!(physical_value(&s, &ty, Bounds::I64), Ok(exact));
        let mut l = Vec::new();
        textio::write_physical(&mut l, exact, &ty, Side::Right, 0, unit).unwrap();
        prop_assert_eq!(textio::read_physical(&mut &l[..], &ty, Bounds::I64), Ok(exact));
    }

    #[test]
    fn enumeration(lits in prop::collection::vec(ident(), 1..8).prop_filter("distinct", |l| distinct(l))) {
        let ty = EnumType { lits };
        for (pos, lit) in ty.lits.iter().enumerate() {
            prop_assert_eq!(enum_value(&enum_image(lit), &ty), Ok(pos));
        }
    }

    #[test]
    fn bit_strings(bits in prop::collection::vec(any::<bool>(), 0..40)) {
        let n = bits.len();
        let mut l = Vec::new();
        textio::hwrite(&mut l, &bits, Side::Right, 0);
        textio::write_character(&mut l, b' ', Side::Right, 0);
        textio::owrite(&mut l, &bits, Side::Right, 0);
        textio::write_bit_vector(&mut l, &bits, Side::Right, n + 2);
        let mut l = &l[..];
        prop_assert_eq!(textio::hread(&mut l, n), Ok(bits.clone()));
        prop_assert_eq!(textio::oread(&mut l, n), Ok(bits.clone()));
        prop_assert_eq!(textio::read_bit_vector(&mut l, n), Ok(bits.clone()));
        prop_assert!(l.iter().all(|&c| c == b' '));
    }

    #[test]
    fn never_panics(s in prop::collection::vec(any::<u8>(), 0..24)) {
        let ty = time_type();
        let _ = integer_value(&s, Bounds::I64);
        let _ = real_value(&s, RealBounds::REAL);
        let _ = physical_value(&s, &ty, Bounds::I64);
        let _ = enum_value(&s, &character_type());
        let _ = real_to_string_format(1.5, &s);
        let _ = textio::read_integer(&mut &s[..], Bounds::I32);
        let _ = textio::read_real(&mut &s[..], RealBounds::REAL);
        let _ = textio::read_physical(&mut &s[..], &ty, Bounds::I64);
        let _ = textio::read_boolean(&mut &s[..]);
        let _ = textio::hread(&mut &s[..], 7);
        let _ = textio::oread(&mut &s[..], 7);
        let _ = textio::sread(&mut &s[..], 5);
    }
}

#[test]
fn every_character() {
    for c in 0..=255u8 {
        assert_eq!(character_value(&character_image(c)), Ok(c), "{c}");
    }
}
