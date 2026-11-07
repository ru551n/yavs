//! The lexer: preprocessed bytes to tokens (IEEE 1364-2005 clause 3).

use crate::Diag;
use crate::pre::{ENVELOPE, Pre};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum K {
    /// Simple or escaped identifier (text without the backslash of an escaped one).
    Ident,
    Kw,
    /// `$name`
    Sys,
    /// Integer or based literal, source text including the size.
    Num,
    Real,
    /// String literal, text includes the quotes.
    Str,
    /// Punctuation or operator; `s` holds it.
    P,
    /// A removed protected envelope; `n` is its index.
    Env,
    Eof,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Tok {
    pub k: K,
    /// The keyword or punctuation text; empty for the other kinds.
    pub s: &'static str,
    pub start: u32,
    pub end: u32,
    pub n: u32,
}

/// Reserved words of IEEE 1364-2005 (without the words that only mean something inside `config`).
const KEYWORDS: &[&str] = &[
    "always",
    "and",
    "assign",
    "automatic",
    "begin",
    "buf",
    "bufif0",
    "bufif1",
    "case",
    "casex",
    "casez",
    "cmos",
    "config",
    "deassign",
    "default",
    "defparam",
    "disable",
    "edge",
    "else",
    "end",
    "endcase",
    "endconfig",
    "endfunction",
    "endgenerate",
    "endmodule",
    "endprimitive",
    "endspecify",
    "endtable",
    "endtask",
    "event",
    "for",
    "force",
    "forever",
    "fork",
    "function",
    "generate",
    "genvar",
    "highz0",
    "highz1",
    "if",
    "ifnone",
    "initial",
    "inout",
    "input",
    "integer",
    "join",
    "large",
    "localparam",
    "macromodule",
    "medium",
    "module",
    "nand",
    "negedge",
    "nmos",
    "nor",
    "noshowcancelled",
    "not",
    "notif0",
    "notif1",
    "or",
    "output",
    "parameter",
    "pmos",
    "posedge",
    "primitive",
    "pull0",
    "pull1",
    "pulldown",
    "pullup",
    "pulsestyle_ondetect",
    "pulsestyle_onevent",
    "rcmos",
    "real",
    "realtime",
    "reg",
    "release",
    "repeat",
    "rnmos",
    "rpmos",
    "rtran",
    "rtranif0",
    "rtranif1",
    "scalared",
    "showcancelled",
    "signed",
    "small",
    "specify",
    "specparam",
    "strong0",
    "strong1",
    "supply0",
    "supply1",
    "table",
    "task",
    "time",
    "tran",
    "tranif0",
    "tranif1",
    "tri",
    "tri0",
    "tri1",
    "triand",
    "trior",
    "trireg",
    "unsigned",
    "uwire",
    "vectored",
    "wait",
    "wand",
    "weak0",
    "weak1",
    "while",
    "wire",
    "wor",
    "xnor",
    "xor",
];

/// Operators, longest first within the same first byte.
const PUNCT: &[&str] = &[
    "<<<=", ">>>=", "===", "!==", "<<<", ">>>", "&&&", "<<=", ">>=", "==?", "!=?", "|->", "|=>",
    "<->", "->>", "==", "!=", "&&", "||", "<=", ">=", "<<", ">>", "**", "~&", "~|", "~^", "^~",
    "+:", "-:", "=>", "*>", "->", "++", "--", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "::",
    "##", "(*", "*)", "(", ")", "[", "]", "{", "}", ";", ",", ".", ":", "#", "@", "?", "=", "!",
    "~", "&", "|", "^", "+", "-", "*", "/", "%", "<", ">", "$", "'",
];

pub(crate) fn keyword(w: &[u8]) -> Option<&'static str> {
    let s = std::str::from_utf8(w).ok()?;
    KEYWORDS.binary_search(&s).ok().map(|i| KEYWORDS[i])
}

fn is_id(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$'
}

fn is_ws(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\r' | b'\n' | 0x0c)
}

pub(crate) fn lex(pre: &Pre) -> (Vec<Tok>, Vec<Diag>) {
    let b = &pre.text[..];
    let n = b.len();
    let mut toks: Vec<Tok> = Vec::with_capacity(n / 4);
    let mut diags = Vec::new();
    let mut i = 0;
    let mut envs = 0;
    let mut in_attr = false;
    macro_rules! err {
        ($code:literal, $a:expr, $z:expr, $($m:tt)*) => {
            diags.push(Diag::error($code, format!($($m)*), pre.span($a as u32, $z as u32)))
        };
    }
    while i < n {
        let c = b[i];
        if is_ws(c) {
            i += 1;
            continue;
        }
        let start = i;
        macro_rules! tok {
            ($k:expr, $s:expr, $end:expr, $nn:expr) => {
                toks.push(Tok {
                    k: $k,
                    s: $s,
                    start: start as u32,
                    end: $end as u32,
                    n: $nn,
                })
            };
        }
        if c == ENVELOPE {
            tok!(K::Env, "", i + 1, envs);
            envs += 1;
            i += 1;
        } else if c.is_ascii_alphabetic() || c == b'_' {
            let mut j = i;
            while j < n && is_id(b[j]) {
                j += 1;
            }
            match keyword(&b[i..j]) {
                Some(k) => tok!(K::Kw, k, j, 0),
                None => tok!(K::Ident, "", j, 0),
            }
            i = j;
            // The symbols of a UDP table are not Verilog tokens: the parser reads them as text.
            if toks.last().is_some_and(|t| t.s == "table") {
                let mut k = i;
                while k < n && !(b[k..].starts_with(b"endtable") && (k == 0 || !is_id(b[k - 1]))) {
                    k += 1;
                }
                i = k;
            }
        } else if c == b'\\' {
            let mut j = i + 1;
            while j < n && !is_ws(b[j]) {
                j += 1;
            }
            if j == i + 1 {
                err!("E4014", i, i + 1, "empty escaped identifier");
            }
            tok!(K::Ident, "", j, 1);
            i = j;
        } else if c == b'$' && i + 1 < n && (b[i + 1].is_ascii_alphabetic() || b[i + 1] == b'_') {
            let mut j = i + 1;
            while j < n && is_id(b[j]) {
                j += 1;
            }
            tok!(K::Sys, "", j, 0);
            i = j;
        } else if c == b'"' {
            let mut j = i + 1;
            loop {
                if j >= n || b[j] == b'\n' {
                    err!("E4014", i, j.min(n).max(i + 1), "unterminated string");
                    break;
                }
                if b[j] == b'\\' {
                    j += 2;
                    continue;
                }
                if b[j] == b'"' {
                    j += 1;
                    break;
                }
                j += 1;
            }
            let j = j.min(n);
            tok!(K::Str, "", j, 0);
            i = j;
        } else if c.is_ascii_digit() || (c == b'\'' && based_ahead(b, i + 1)) {
            let after_hash = toks.last().is_some_and(|t| t.k == K::P && t.s == "#");
            let (end, real) = number(b, i, after_hash);
            tok!(if real { K::Real } else { K::Num }, "", end, 0);
            let bad = based_problem(&b[i..end]);
            if let Some(why) = bad {
                err!("E4014", i, end, "{why}");
            }
            // A letter or digit right after the literal is a suffix or a digit the base does not allow, not a new token.
            if end < n && (b[end].is_ascii_alphanumeric() || b[end] == b'_') {
                let mut j = end;
                while j < n && is_id(b[j]) {
                    j += 1;
                }
                let suf = &b[end..j];
                if matches!(suf, b"s" | b"ms" | b"us" | b"ns" | b"ps" | b"fs") {
                    err!(
                        "E4003",
                        i,
                        j,
                        "time literal `{}` (SystemVerilog)",
                        String::from_utf8_lossy(&b[i..j])
                    );
                } else if bad.is_none() {
                    err!(
                        "E4014",
                        i,
                        j,
                        "malformed numeric literal `{}`",
                        String::from_utf8_lossy(&b[i..j])
                    );
                }
                i = j;
            } else {
                i = end;
            }
        } else {
            // `(*)` is a parenthesised star, not an attribute start.
            if c == b'(' && b.get(i + 1) == Some(&b'*') && b.get(i + 2) == Some(&b')') {
                tok!(K::P, "(", i + 1, 0);
                toks.push(Tok {
                    k: K::P,
                    s: "*",
                    start: (i + 1) as u32,
                    end: (i + 2) as u32,
                    n: 0,
                });
                toks.push(Tok {
                    k: K::P,
                    s: ")",
                    start: (i + 2) as u32,
                    end: (i + 3) as u32,
                    n: 0,
                });
                i += 3;
                continue;
            }
            let mut hit = None;
            for p in PUNCT {
                // `*)` only closes an attribute: `@( *)` is a star list.
                if *p == "*)" && !in_attr {
                    continue;
                }
                if b[i..].starts_with(p.as_bytes()) {
                    hit = Some(*p);
                    break;
                }
            }
            match hit {
                Some(p) => {
                    match p {
                        // `@(* )` is a star list, not an attribute.
                        "(*" => in_attr = b[i + 2..].iter().find(|c| !is_ws(**c)) != Some(&b')'),
                        "*)" => in_attr = false,
                        _ => {}
                    }
                    tok!(K::P, p, i + p.len(), 0);
                    i += p.len();
                }
                None => {
                    err!(
                        "E4014",
                        i,
                        i + 1,
                        "illegal character `{}`",
                        String::from_utf8_lossy(&b[i..(i + 1).min(n)]).escape_debug()
                    );
                    i += 1;
                    while i < n && (b[i] & 0xC0) == 0x80 {
                        i += 1;
                    }
                }
            }
        }
    }
    toks.push(Tok {
        k: K::Eof,
        s: "",
        start: n as u32,
        end: n as u32,
        n: 0,
    });
    (toks, diags)
}

/// What is wrong with a based literal (clause 3.5.1), `None` for a good one and for a literal that is not based.
fn based_problem(lit: &[u8]) -> Option<&'static str> {
    let q = lit.iter().position(|&c| c == b'\'')?;
    let size = &lit[..q];
    if !size.is_empty() && size.iter().all(|&c| c == b'0' || c == b'_') {
        return Some("the size of a constant is not zero (clause 3.5.1)");
    }
    let mut r = &lit[q + 1..];
    if matches!(r.first(), Some(b's' | b'S')) {
        r = &r[1..];
    }
    let base = r.first()?.to_ascii_lowercase();
    let digits: Vec<u8> = r[1..]
        .iter()
        .copied()
        .skip_while(|c| c.is_ascii_whitespace())
        .collect();
    if digits.is_empty() {
        return Some("a based constant has at least one digit (clause 3.5.1)");
    }
    if digits[0] == b'_' {
        return Some("the digits of a constant do not start with an underscore (clause 3.5.1)");
    }
    let xz = |c: &u8| matches!(c, b'x' | b'X' | b'z' | b'Z' | b'?');
    if base == b'd' && digits.iter().any(xz) && digits.iter().filter(|&&c| c != b'_').count() > 1 {
        return Some("x or z in a decimal constant is the only digit (clause 3.5.1)");
    }
    None
}

/// After `'`: `[sS]? [bBoOdDhH]`.
fn based_ahead(b: &[u8], i: usize) -> bool {
    let mut j = i;
    if matches!(b.get(j), Some(b's' | b'S')) {
        j += 1;
    }
    matches!(
        b.get(j),
        Some(b'b' | b'B' | b'o' | b'O' | b'd' | b'D' | b'h' | b'H')
    )
}

/// Scan a number at `i` (a digit or the `'` of an unsized based literal); returns the end and whether it is real.
fn number(b: &[u8], i: usize, after_hash: bool) -> (usize, bool) {
    let n = b.len();
    let mut j = i;
    if b[j] != b'\'' {
        while j < n && (b[j].is_ascii_digit() || b[j] == b'_') {
            j += 1;
        }
        // Size followed by a base, whitespace allowed in between.
        let mut k = j;
        while k < n && matches!(b[k], b' ' | b'\t' | b'\n' | b'\r') {
            k += 1;
        }
        // A size is never zero (clause 3.5.1), and a number after `#` is a delay: `#0 'bz` and `#1 'd0`
        // are a delay and an unsized literal.
        let zero = b[i..j].iter().all(|&c| c == b'0' || c == b'_');
        if k < n && b[k] == b'\'' && based_ahead(b, k + 1) && !((zero || after_hash) && k > j) {
            j = k;
        } else {
            // Real: fraction and/or exponent.
            let mut real = false;
            let mut m = j;
            if m + 1 < n && b[m] == b'.' && b[m + 1].is_ascii_digit() {
                m += 1;
                while m < n && (b[m].is_ascii_digit() || b[m] == b'_') {
                    m += 1;
                }
                real = true;
            }
            if m < n && matches!(b[m], b'e' | b'E') {
                let mut e = m + 1;
                if e < n && matches!(b[e], b'+' | b'-') {
                    e += 1;
                }
                if e < n && b[e].is_ascii_digit() {
                    while e < n && (b[e].is_ascii_digit() || b[e] == b'_') {
                        e += 1;
                    }
                    m = e;
                    real = true;
                }
            }
            return (m, real);
        }
    }
    // b[j] == '\''
    j += 1;
    if matches!(b.get(j), Some(b's' | b'S')) {
        j += 1;
    }
    let base = b[j].to_ascii_lowercase();
    j += 1;
    while j < n && matches!(b[j], b' ' | b'\t' | b'\n' | b'\r') {
        j += 1;
    }
    let ok = |c: u8| match base {
        b'b' => matches!(c, b'0' | b'1'),
        b'o' => matches!(c, b'0'..=b'7'),
        b'd' => c.is_ascii_digit(),
        _ => c.is_ascii_hexdigit(),
    };
    while j < n && (ok(b[j]) || matches!(b[j], b'x' | b'X' | b'z' | b'Z' | b'?' | b'_')) {
        j += 1;
    }
    (j, false)
}
