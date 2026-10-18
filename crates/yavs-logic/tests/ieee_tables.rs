//! Every table entry against the IEEE std_logic_1164 package body (the spec).
//! Uses the vendored copy in lib/ieee/2008.

use std::fs;
use yavs_logic::{ALL, StdULogic, tables};

fn ieee(file: &str) -> String {
    let p = format!("{}/../../lib/ieee/2008/{file}", env!("CARGO_MANIFEST_DIR"));
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {p}: {e}"))
}

/// The quoted characters (before any `--` comment) of the table starting at `constant <name>`.
fn parse(src: &str, name: &str) -> Vec<char> {
    let start = src
        .find(&format!("constant {name} "))
        .unwrap_or_else(|| panic!("{name} missing"));
    let mut out = vec![];
    for line in src[start..].lines().skip(1) {
        let code = line.split("--").next().unwrap();
        let mut it = code.chars();
        while let Some(c) = it.next() {
            if c == '\'' {
                out.push(it.next().unwrap());
                it.next();
            }
        }
        if code.contains(");") {
            break;
        }
    }
    out
}

fn check2(src: &str, name: &str, t: &[[u8; 16]; 16]) {
    let spec = parse(src, name);
    assert_eq!(spec.len(), 81, "{name}");
    for (i, a) in ALL.iter().enumerate() {
        for (j, b) in ALL.iter().enumerate() {
            let got = StdULogic::from_code_pub(t[i][j]);
            assert_eq!(got.to_char(), spec[i * 9 + j], "{name}[{a}][{b}]");
        }
    }
}

fn check1(src: &str, name: &str, t: &[u8; 16]) {
    let spec = parse(src, name);
    assert_eq!(spec.len(), 9, "{name}");
    for (i, a) in ALL.iter().enumerate() {
        assert_eq!(
            StdULogic::from_code_pub(t[i]).to_char(),
            spec[i],
            "{name}[{a}]"
        );
    }
}

#[test]
fn tables_match_ieee_body() {
    let src = ieee("std_logic_1164-body.vhdl");
    check2(&src, "resolution_table", &tables::RESOLVE);
    check2(&src, "and_table", &tables::AND);
    check2(&src, "or_table", &tables::OR);
    check2(&src, "xor_table", &tables::XOR);
    check1(&src, "not_table", &tables::NOT);
    check1(&src, "cvt_to_x01", &tables::TO_X01);
    check1(&src, "cvt_to_x01z", &tables::TO_X01Z);
    check1(&src, "cvt_to_ux01", &tables::TO_UX01);
}

/// numeric_std MATCH_TABLE is the boolean image of the `?=` table (`1` = true).
#[test]
fn match_table_vs_numeric_std() {
    let src = ieee("numeric_std-body.vhdl");
    let start = src.find("constant MATCH_TABLE").unwrap();
    let mut spec = vec![];
    for line in src[start..].lines().skip(1) {
        let code = line.split("--").next().unwrap();
        spec.extend(
            code.split(|c: char| !c.is_alphabetic())
                .filter(|w| *w == "true" || *w == "false")
                .map(|w| w == "true"),
        );
        if code.contains(");") {
            break;
        }
    }
    assert_eq!(spec.len(), 81);
    for (i, &a) in ALL.iter().enumerate() {
        for (j, &b) in ALL.iter().enumerate() {
            // std_match is true exactly when `?=` yields '1'.
            assert_eq!(a.match_eq(b) == StdULogic::One, spec[i * 9 + j], "{a}{b}");
        }
    }
}
