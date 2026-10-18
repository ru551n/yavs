//! Generates the property-test cases for `tests/generated.rs`: Rust
//! `extern "C"` callees with random signatures (fixed seed) plus all
//! signatures of up to two parameters, each with a check that calls the
//! callee directly and through the trampoline and compares results.
//! Only the tests include the output.

use std::fmt::Write;

const TYPES: [&str; 10] = [
    "I8", "I16", "I32", "I64", "U8", "U16", "U32", "F32", "F64", "Ptr",
];

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn rust_type(t: &str) -> &'static str {
    match t {
        "I8" => "i8",
        "I16" => "i16",
        "I32" => "i32",
        "I64" => "i64",
        "U8" => "u8",
        "U16" => "u16",
        "U32" => "u32",
        "F32" => "f32",
        "F64" => "f64",
        _ => "*mut c_void",
    }
}

/// A literal of type `t`, biased towards the extremes.
fn literal(rng: &mut Rng, t: &str) -> String {
    let bits = match rng.below(4) {
        0 => u64::MAX,
        1 => 1 << 63,
        2 => 0x7fff_ffff_ffff_ffff,
        _ => rng.next(),
    };
    match t {
        "F32" => format!("f32::from_bits({:#x})", bits as u32),
        "F64" => format!("f64::from_bits({bits:#x})"),
        "Ptr" => format!("({bits:#x}_usize as *mut c_void)"),
        _ => format!("({bits:#x}_u64 as {})", rust_type(t)),
    }
}

fn ret_expr(t: &str) -> &'static str {
    match t {
        "F32" => "(h >> 40) as f32",
        "F64" => "(h >> 11) as f64",
        "Ptr" => "h as usize as *mut c_void",
        "I8" => "h as i8",
        "I16" => "h as i16",
        "I32" => "h as i32",
        "I64" => "h as i64",
        "U8" => "h as u8",
        "U16" => "h as u16",
        _ => "h as u32",
    }
}

fn case(out: &mut String, n: usize, params: &[&str], ret: Option<&str>, rng: &mut Rng) {
    let decl: Vec<String> = params
        .iter()
        .enumerate()
        .map(|(i, t)| format!("a{i}: {}", rust_type(t)))
        .collect();
    let ret_ty = ret.map_or(String::new(), |t| format!(" -> {}", rust_type(t)));
    writeln!(out, "extern \"C\" fn g{n}({}){ret_ty} {{", decl.join(", ")).unwrap();
    writeln!(out, "    let mut h = SEED;").unwrap();
    for i in 0..params.len() {
        writeln!(out, "    h = step(h, a{i}.mix());").unwrap();
    }
    writeln!(out, "    set_last(h);").unwrap();
    if let Some(t) = ret {
        writeln!(out, "    {}", ret_expr(t)).unwrap();
    }
    writeln!(out, "}}").unwrap();

    let lits: Vec<String> = params.iter().map(|t| literal(rng, t)).collect();
    let args: Vec<String> = params
        .iter()
        .zip(&lits)
        .map(|(t, l)| format!("Arg::{t}({l})"))
        .collect();
    let types: Vec<String> = params.iter().map(|t| format!("CType::{t}")).collect();
    let direct = format!("g{n}({})", lits.join(", "));
    let expect = match ret {
        Some(t) => format!("Ret::{t}({direct})"),
        None => format!("{{ {direct}; Ret::Void }}"),
    };
    let ret_sig = ret.map_or("None".into(), |t| format!("Some(CType::{t})"));
    writeln!(
        out,
        "fn c{n}() {{\n    let want = {expect};\n    let want_h = last();\n    \
         let sig = Signature {{ params: vec![{}], ret: {ret_sig} }};\n    \
         let got = unsafe {{ call(g{n} as *const c_void, &sig, &[{}]) }}.unwrap();\n    \
         assert_eq!(got, want, \"g{n} result\");\n    \
         assert_eq!(last(), want_h, \"g{n} arguments\");\n}}",
        types.join(", "),
        args.join(", ")
    )
    .unwrap();
}

fn main() {
    let mut rng = Rng(0x5eed_f00d_1234_5678);
    let mut out = String::new();
    let mut n = 0;
    let rets: Vec<Option<&str>> = std::iter::once(None)
        .chain(TYPES.iter().map(|&t| Some(t)))
        .collect();

    // Every signature of up to two parameters, results cycling.
    let mut sigs: Vec<Vec<&str>> = vec![vec![]];
    for &a in &TYPES {
        sigs.push(vec![a]);
        for &b in &TYPES {
            sigs.push(vec![a, b]);
        }
    }
    for params in &sigs {
        case(&mut out, n, params, rets[n % rets.len()], &mut rng);
        n += 1;
    }

    // Random signatures up to 40 parameters, a few at the 64 maximum,
    // with integer-heavy, mixed and float-heavy type mixes.
    for i in 0..320 {
        let arity = if i % 32 == 0 {
            64
        } else {
            rng.below(41) as usize
        };
        let float_pct = [10, 50, 90][i % 3];
        let params: Vec<&str> = (0..arity)
            .map(|_| {
                if rng.below(100) < float_pct {
                    TYPES[7 + rng.below(2) as usize]
                } else if rng.below(10) == 0 {
                    "Ptr"
                } else {
                    TYPES[rng.below(7) as usize]
                }
            })
            .collect();
        let ret = rets[rng.below(rets.len() as u64) as usize];
        case(&mut out, n, &params, ret, &mut rng);
        n += 1;
    }

    let list: Vec<String> = (0..n).map(|i| format!("c{i}")).collect();
    writeln!(out, "pub const CASES: &[fn()] = &[{}];", list.join(", ")).unwrap();
    let path = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("gen_cases.rs");
    std::fs::write(path, out).unwrap();
    println!("cargo::rerun-if-changed=build.rs");
}
