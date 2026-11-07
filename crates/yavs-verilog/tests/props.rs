//! Property tests of the front end: number literals, macros and conditionals, and robustness against damaged input.

use std::path::{Path, PathBuf};

use proptest::prelude::*;
use yavs_verilog::{Options, Severity, analyze, parse_str};

fn errors(src: &str, opts: &Options) -> Vec<&'static str> {
    match parse_str(src, opts) {
        Ok(_) => Vec::new(),
        Err(d) => d
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .map(|d| d.code)
            .collect(),
    }
}

/// The digits a base allows, besides x, z and ? (decimal takes those alone).
fn digit_set(base: char) -> &'static str {
    match base {
        'b' => "01",
        'o' => "01234567",
        'd' => "0123456789",
        _ => "0123456789abcdefABCDEF",
    }
}

fn literal() -> impl Strategy<Value = (String, char, String)> {
    (
        1u32..300,
        prop::sample::select(vec!['b', 'o', 'd', 'h', 'B', 'O', 'D', 'H']),
        any::<bool>(),
        prop::collection::vec(any::<prop::sample::Index>(), 1..40),
        any::<bool>(),
        prop::sample::select(vec!["", " ", "\n  "]),
    )
        .prop_map(|(size, base, signed, idx, xz, gap)| {
            let b = base.to_ascii_lowercase();
            let set: Vec<char> = digit_set(b).chars().collect();
            let mut digits: String = idx.iter().map(|i| set[i.index(set.len())]).collect();
            if xz && b != 'd' {
                digits.push('x');
                digits.push_str("_z?");
            }
            let s = if signed { "s" } else { "" };
            (format!("{size}{gap}'{s}{base}{gap}{digits}"), b, digits)
        })
}

proptest! {
    /// Clause 3.5.1: every sized or unsized, signed or unsigned constant of a base with its own digits is one number.
    #[test]
    fn based_constants_parse((lit, _, _) in literal()) {
        let src = format!("module m; parameter p = {lit}; endmodule");
        prop_assert_eq!(errors(&src, &Options::default()), Vec::<&str>::new(), "{}", lit);
    }

    /// A digit the base does not allow is one lexical error, never a syntax error as well.
    #[test]
    fn a_foreign_digit_is_one_lexical_error(
        size in 1u32..100, base in prop::sample::select(vec!['b', 'o']), at in any::<prop::sample::Index>(),
    ) {
        let bad = if base == 'b' { '2' } else { '8' };
        let mut digits: Vec<char> = digit_set(base).chars().collect();
        let k = at.index(digits.len() + 1);
        digits.insert(k.max(1), bad);
        let lit: String = format!("{size}'{base}{}", digits.iter().collect::<String>());
        let src = format!("module m; parameter p = {lit}; endmodule");
        prop_assert_eq!(errors(&src, &Options::default()), vec!["E4014"], "{}", lit);
    }

    /// Clause 3.5.2: a real has digits on both sides of the point, or an exponent.
    #[test]
    fn reals_parse(int in "[0-9]{1,6}", frac in "[0-9]{1,6}", exp in prop::option::of(("[eE]", "[+-]?", "[0-9]{1,3}"))) {
        let e = exp.map(|(a, b, c)| format!("{a}{b}{c}")).unwrap_or_default();
        let src = format!("module m; real r = {int}.{frac}{e}; real s = {int}{}; endmodule", if e.is_empty() { "e1".into() } else { e.clone() });
        prop_assert_eq!(errors(&src, &Options::default()), Vec::<&str>::new(), "{}", src);
    }
}

fn scratch(tag: &str) -> PathBuf {
    let d =
        Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("props-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A tree of conditionals over the macros A, B and C, with a module in some leaves.
#[derive(Clone, Debug)]
enum Node {
    Module,
    Cond {
        ndef: bool,
        name: u8,
        then: Vec<Node>,
        els: Option<Vec<Node>>,
    },
}

fn tree() -> impl Strategy<Value = Vec<Node>> {
    let leaf = Just(Node::Module);
    let node = leaf.prop_recursive(4, 24, 3, |inner| {
        (
            any::<bool>(),
            0u8..3,
            prop::collection::vec(inner.clone(), 0..3),
            prop::option::of(prop::collection::vec(inner, 0..3)),
        )
            .prop_map(|(ndef, name, then, els)| Node::Cond {
                ndef,
                name,
                then,
                els,
            })
    });
    prop::collection::vec(node, 1..5)
}

fn render(nodes: &[Node], n: &mut usize, out: &mut String) {
    for node in nodes {
        match node {
            Node::Module => {
                out.push_str(&format!("module u{n};\nendmodule\n"));
                *n += 1;
            }
            Node::Cond {
                ndef,
                name,
                then,
                els,
            } => {
                let nm = ['A', 'B', 'C'][*name as usize];
                out.push_str(&format!(
                    "`{} {nm}\n",
                    if *ndef { "ifndef" } else { "ifdef" }
                ));
                render(then, n, out);
                if let Some(e) = els {
                    out.push_str("`else\n");
                    render(e, n, out);
                }
                out.push_str("`endif\n");
            }
        }
    }
}

/// The modules whose text is compiled, by the rules of clause 19.4, numbered as `render` numbers them.
fn active(nodes: &[Node], defined: &[bool; 3], on: bool, n: &mut usize, out: &mut Vec<String>) {
    for node in nodes {
        match node {
            Node::Module => {
                if on {
                    out.push(format!("u{n}"));
                }
                *n += 1;
            }
            Node::Cond {
                ndef,
                name,
                then,
                els,
            } => {
                let cond = defined[*name as usize] != *ndef;
                active(then, defined, on && cond, n, out);
                if let Some(e) = els {
                    active(e, defined, on && !cond, n, out);
                }
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Clause 19.4: nested conditionals compile exactly the branches the defined macros select; dropping an `endif` is E4004.
    #[test]
    fn conditionals_select_the_right_modules(t in tree(), defs in prop::array::uniform3(any::<bool>())) {
        let (mut src, mut n) = (String::new(), 0);
        render(&t, &mut n, &mut src);
        let dir = scratch("cond");
        let f = dir.join("t.v");
        std::fs::write(&f, format!("`timescale 1ns/1ps\n{src}")).unwrap();
        let mut o = Options::default();
        for (i, d) in defs.iter().enumerate() {
            if *d {
                o.defines.push((["A", "B", "C"][i].into(), None));
            }
        }
        let r = analyze(std::slice::from_ref(&f), &o);
        prop_assert!(r.diagnostics.iter().all(|d| d.severity != Severity::Error), "{:?}", r.diagnostics);
        let (mut want, mut k) = (Vec::new(), 0);
        active(&t, &defs, true, &mut k, &mut want);
        let got: Vec<&str> = r.units.iter().map(|u| u.name.as_str()).collect();
        prop_assert_eq!(got, want);
        if let Some(i) = src.rfind("`endif") {
            std::fs::write(&f, format!("`timescale 1ns/1ps\n{}", &src[..i])).unwrap();
            let r = analyze(std::slice::from_ref(&f), &o);
            prop_assert!(r.diagnostics.iter().any(|d| d.code == "E4004"));
        }
    }

    /// Clause 19.3: a chain of macros that use the one before expands to a constant; wrong argument counts are E4008.
    #[test]
    fn macro_chains_and_argument_counts(depth in 1usize..40, formals in 1usize..8, given in 1usize..10) {
        let mut src = String::from("`define M0 1\n");
        for k in 1..=depth {
            src.push_str(&format!("`define M{k} (`M{} + 1)\n", k - 1));
        }
        let names: Vec<String> = (0..formals).map(|i| format!("a{i}")).collect();
        src.push_str(&format!("`define F({}) ({})\n", names.join(", "), names.join(" + ")));
        let args: Vec<String> = (0..given).map(|i| i.to_string()).collect();
        src.push_str(&format!("module m; parameter p = `M{depth}; parameter q = `F({}); endmodule\n", args.join(", ")));
        let e = errors(&src, &Options::default());
        if given == formals {
            prop_assert_eq!(e, Vec::<&str>::new());
        } else {
            prop_assert_eq!(e, vec!["E4008"]);
        }
    }
}

fn corpus() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/verilog");
    let mut v = Vec::new();
    for d in ["ci", "lrm"] {
        let mut names: Vec<_> = std::fs::read_dir(root.join(d))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        names.sort();
        for p in names {
            if p.extension().is_some_and(|e| e == "v") {
                v.push((
                    p.file_name().unwrap().to_string_lossy().into_owned(),
                    std::fs::read_to_string(&p).unwrap(),
                ));
            }
        }
    }
    v
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]

    /// Truncated, cut and corrupted sources end in diagnostics, never a panic, and every span lies inside its file.
    #[test]
    fn damaged_sources_are_diagnosed(
        which in any::<prop::sample::Index>(), a in any::<prop::sample::Index>(), b in any::<prop::sample::Index>(),
        mode in 0u8..3, byte in prop::sample::select(b"`\"'/*()[]{};:#@$\\ \n(*)".to_vec()),
    ) {
        let c = corpus();
        let (name, text) = &c[which.index(c.len())];
        let mut bytes = text.as_bytes().to_vec();
        let (i, j) = (a.index(bytes.len() + 1), b.index(bytes.len() + 1));
        let (i, j) = (i.min(j), i.max(j));
        match mode {
            0 => bytes.truncate(i),
            1 => { bytes.drain(i..j.min(i + 40)); }
            _ => if i < bytes.len() { bytes[i] = byte },
        }
        let dir = scratch("damage");
        let f = dir.join("t.v");
        std::fs::write(&f, &bytes).unwrap();
        let mut o = Options::default();
        o.include_dirs.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/verilog/lrm/inc"));
        let r = analyze(std::slice::from_ref(&f), &o);
        for d in &r.diagnostics {
            if let Some(s) = d.span {
                let len = std::fs::metadata(&r.files[s.file]).map(|m| m.len() as u32).unwrap_or(0);
                prop_assert!(s.start <= s.end && s.end <= len.max(bytes.len() as u32), "{name}: span {s:?} of {}", d.code);
            }
        }
        if r.diagnostics.iter().all(|d| d.severity != Severity::Error) && r.stopped_at.is_some() {
            prop_assert!(false, "{name}: stopped without an error");
        }
    }
}
