//! Native `ieee` operators against their VHDL bodies (`yavs_ir::ieee`).
//!
//! A generated testbench applies every native operator to random operands, metavalues, null
//! arrays, odd lengths and both directions, and reports each result with its bounds. It runs
//! twice, natives on and off (`YAVS_NO_NATIVE_IEEE`); stdout, stderr and the exit code must be
//! identical. `YAVS_IEEE_KEEP=dir` keeps the generated sources.

mod common;
use common::tempdir;

use std::fmt::Write;
use std::path::Path;
use std::process::{Command, Output};

/// Iterations of a generator loop: `YAVS_IEEE_SCALE=4` runs a quarter (the compiled tier needs
/// minutes for the full straight-line testbenches).
fn iters(n: usize) -> usize {
    (n / scale()).max(1)
}

fn scale() -> usize {
    std::env::var("YAVS_IEEE_SCALE")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|d| *d > 0)
        .unwrap_or(1)
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

const HEAD: &str = r#"library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity t is end entity;
architecture a of t is
  function img (x : std_ulogic_vector) return string is
    variable r : string(1 to x'length);
    variable k : integer := 0;
  begin
    for i in x'range loop
      k := k + 1;
      r(k) := std_ulogic'image(x(i))(2);
    end loop;
    return integer'image(x'left) & "," & integer'image(x'right) & "," &
           boolean'image(x'ascending) & ":" & r;
  end function;
  function img (x : unsigned) return string is
  begin return img(std_ulogic_vector(x)); end function;
  function img (x : signed) return string is
  begin return img(std_ulogic_vector(x)); end function;
  function img (x : bit_vector) return string is
    variable r : string(1 to x'length);
    variable k : integer := 0;
  begin
    for i in x'range loop
      k := k + 1;
      r(k) := bit'image(x(i))(2);
    end loop;
    return integer'image(x'left) & "," & integer'image(x'right) & "," &
           boolean'image(x'ascending) & ":" & r;
  end function;
  function img (b : boolean) return string is
  begin return boolean'image(b); end function;
  function img (s : std_ulogic) return string is
  begin return std_ulogic'image(s); end function;
  function img (i : integer) return string is
  begin return integer'image(i); end function;
  function cv (c : character) return std_ulogic is
  begin
    case c is
      when 'U' => return 'U'; when 'X' => return 'X'; when '0' => return '0';
      when '1' => return '1'; when 'Z' => return 'Z'; when 'W' => return 'W';
      when 'L' => return 'L'; when 'H' => return 'H'; when others => return '-';
    end case;
  end function;
  function sv (s : string; l : integer; asc : boolean) return std_ulogic_vector is
    variable d : std_ulogic_vector(l downto l - s'length + 1);
    variable u : std_ulogic_vector(l to l + s'length - 1);
  begin
    if asc then
      for i in u'range loop u(i) := cv(s(s'left + i - l)); end loop;
      return u;
    else
      for i in d'range loop d(i) := cv(s(s'left + l - i)); end loop;
      return d;
    end if;
  end function;
  function uv (s : string; l : integer; asc : boolean) return unsigned is
  begin return unsigned(sv(s, l, asc)); end function;
  function gv (s : string; l : integer; asc : boolean) return signed is
  begin return signed(sv(s, l, asc)); end function;
begin
  process
  begin
"#;

struct Gen {
    rng: Rng,
    out: String,
    n: usize,
}

impl Gen {
    fn len(&mut self) -> usize {
        match self.rng.below(12) {
            0 => 0,
            1 => 33 + self.rng.below(40) as usize,
            _ => 1 + self.rng.below(12) as usize,
        }
    }
    fn vec(&mut self) -> (String, String) {
        let len = self.len();
        self.vec_n(len)
    }
    fn vec_n(&mut self, len: usize) -> (String, String) {
        // Mostly 0/1, sometimes every std_ulogic value.
        let meta = self.rng.below(3) == 0;
        let s: String = (0..len)
            .map(|_| {
                if meta && self.rng.below(5) == 0 {
                    b"UXZWLH-01"[self.rng.below(9) as usize] as char
                } else {
                    b"01"[self.rng.below(2) as usize] as char
                }
            })
            .collect();
        // Left bounds well above the vector length: the index type is NATURAL.
        let l = self.rng.below(9) as i64 + 97;
        let asc = self.rng.below(2) == 0;
        (s, format!("{l}, {asc}"))
    }
    fn two(&mut self, ty: &str, n: usize) -> (String, String) {
        let (a, b) = (self.vec_n(n), self.vec_n(n));
        (
            format!("{ty}(\"{}\", {})", a.0, a.1),
            format!("{ty}(\"{}\", {})", b.0, b.1),
        )
    }
    /// An operand with no all-zero value: a vector with a `1` (or a metavalue) or a null one.
    fn nonzero(&mut self, ty: &str) -> String {
        let (s, b) = self.vec();
        let s = if s.chars().all(|c| c == '0') && !s.is_empty() {
            format!("1{}", &s[1..])
        } else {
            s
        };
        format!("{ty}(\"{s}\", {b})")
    }
    fn nonzero_bit(&mut self, ty: &str) -> String {
        let len = self.len();
        let mut s: String = (0..len)
            .map(|_| b"01"[self.rng.below(2) as usize] as char)
            .collect();
        if s.chars().all(|c| c == '0') && !s.is_empty() {
            s.replace_range(0..1, "1");
        }
        let (l, asc) = (self.rng.below(9) as i64 + 97, self.rng.below(2) == 0);
        format!("{ty}(\"{s}\", {l}, {asc})")
    }
    fn operand(&mut self, ty: &str) -> String {
        let (s, b) = self.vec();
        format!("{ty}(\"{s}\", {b})")
    }
    fn int(&mut self, nat: bool) -> String {
        let v: i64 = match self.rng.below(6) {
            0 => 0,
            1 => 2147483647,
            2 => self.rng.below(5) as i64,
            3 => self.rng.below(70000) as i64,
            _ => self.rng.below(300) as i64,
        };
        let neg = !nat && self.rng.below(2) == 0;
        if neg {
            format!("(-{v})")
        } else {
            v.to_string()
        }
    }
    fn size(&mut self) -> String {
        match self.rng.below(8) {
            0 => "0".into(),
            1 => "40".into(),
            _ => (1 + self.rng.below(14)).to_string(),
        }
    }
    fn case(&mut self, expr: &str, f: &str) {
        let _ = writeln!(self.out, "    report \"c{} \" & {f}({expr});", self.n);
        self.n += 1;
    }
}

fn testbench(seed: u64, per_op: usize) -> String {
    let mut g = Gen {
        rng: Rng(seed),
        out: String::new(),
        n: 0,
    };
    let sul = "UX01ZWLH-";
    // Scalar tables: every pair.
    for a in sul.chars() {
        for op in ["and", "nand", "or", "nor", "xor", "xnor"] {
            for b in sul.chars() {
                g.case(&format!("'{a}' {op} '{b}'"), "img");
            }
        }
        g.case(&format!("not std_ulogic'('{a}')"), "img");
        for f in ["to_x01", "to_x01z", "to_ux01"] {
            g.case(&format!("{f}(std_ulogic'('{a}'))"), "img");
        }
        g.case(&format!("?? std_ulogic'('{a}')"), "img");
        g.case(&format!("is_x(std_ulogic'('{a}'))"), "img");
    }
    for _ in 0..per_op {
        let sl = ["and", "nand", "or", "nor", "xor", "xnor"];
        for op in sl {
            // Equal lengths (unequal ones end the run with a failure, below).
            let n = g.len();
            let (x, y) = g.two("sv", n);
            g.case(&format!("{x} {op} {y}"), "img");
            // unsigned and signed go through the same operators.
            let (u, v) = g.two("uv", n);
            g.case(&format!("{u} {op} {v}"), "img");
            let (u, v) = g.two("gv", n);
            g.case(&format!("{u} {op} {v}"), "img");
        }
        let n = g.len();
        let (x, y) = g.two("sv", n);
        g.case(&format!("std_match({x}, {y})"), "img");
        let (x, y) = (g.operand("sv"), g.operand("sv"));
        g.case(&format!("std_match({x}, {y})"), "img");
        let x = g.operand("sv");
        g.case(&format!("not {x}"), "img");
        for ty in ["uv", "gv"] {
            let x = g.operand(ty);
            g.case(&format!("not {x}"), "img");
        }
        for f in ["to_x01", "to_x01z", "to_ux01"] {
            g.case(&format!("{f}({x})"), "img");
        }
        g.case(&format!("is_x({x})"), "img");
        g.case(&format!("to_bitvector({x})"), "img");
        g.case(&format!("to_bitvector({x}, '1')"), "img");
        g.case(&format!("to_stdulogicvector(to_bitvector({x}))"), "img");
        // The gates of a vector and a scalar, both sides, every scalar value.
        for op in sl {
            let c = sul.as_bytes()[g.rng.below(9) as usize] as char;
            g.case(&format!("{x} {op} std_ulogic'('{c}')"), "img");
            let c = sul.as_bytes()[g.rng.below(9) as usize] as char;
            g.case(&format!("std_ulogic'('{c}') {op} {x}"), "img");
        }
        // Reductions, also over several plane words.
        let w = if g.rng.below(2) == 0 {
            x.clone()
        } else {
            let len = 60 + g.rng.below(37) as usize;
            let (v, b) = g.vec_n(len);
            format!("sv(\"{v}\", {b})")
        };
        for op in ["and", "nand", "or", "nor", "xor", "xnor"] {
            g.case(&format!("std_ulogic'({op} {w})"), "img");
            g.case(&format!("ieee.std_logic_misc.{op}_reduce({w})"), "img");
            let u = g.operand("uv");
            g.case(&format!("std_ulogic'({op} {u})"), "img");
        }
        let m = sul.chars().nth(g.rng.below(9) as usize).unwrap_or('0');
        g.case(&format!("to_01({x}, '{m}')"), "img");
        for (ty, t) in [("uv", "unsigned"), ("gv", "signed")] {
            let (a, b) = (g.operand(ty), g.operand(ty));
            let b = if g.rng.below(3) == 0 { a.clone() } else { b };
            for op in ["+", "-", "*"] {
                g.case(&format!("{a} {op} {b}"), "img");
            }
            for op in ["<", "<=", ">", ">=", "=", "/="] {
                g.case(&format!("{a} {op} {b}"), "img");
            }
            let nat = t == "unsigned";
            // A zero divisor is the body's error; the generator avoids it.
            let d = g.nonzero(ty);
            for op in ["/", "rem", "mod"] {
                g.case(&format!("{a} {op} {d}"), "img");
            }
            for op in ["+", "-", "*"] {
                let i = g.int(nat);
                g.case(&format!("{a} {op} {i}"), "img");
                let i = g.int(nat);
                g.case(&format!("{i} {op} {a}"), "img");
            }
            for op in ["<", "<=", ">", ">=", "=", "/="] {
                let i = g.int(nat);
                g.case(&format!("{a} {op} {i}"), "img");
                let i = g.int(nat);
                g.case(&format!("{i} {op} {a}"), "img");
            }
            for op in ["?<", "?<=", "?>", "?>=", "?=", "?/="] {
                g.case(&format!("{a} {op} {b}"), "img");
                let i = g.int(nat);
                g.case(&format!("{a} {op} {i}"), "img");
                let i = g.int(nat);
                g.case(&format!("{i} {op} {a}"), "img");
            }
            for f in ["maximum", "minimum"] {
                g.case(&format!("{f}({a}, {b})"), "img");
                let i = g.int(nat);
                g.case(&format!("{f}({a}, {i})"), "img");
                let i = g.int(nat);
                g.case(&format!("{f}({i}, {a})"), "img");
            }
            g.case(&format!("std_match({a}, {b})"), "img");
            let s = g.size();
            g.case(&format!("resize({a}, {s})"), "img");
            let i = g.int(nat);
            let s = g.size();
            let f = if nat { "to_unsigned" } else { "to_signed" };
            g.case(&format!("{f}({i}, {s})"), "img");
            let c = g.rng.below(20);
            for f in ["shift_left", "shift_right", "rotate_left", "rotate_right"] {
                if a.contains("\"\"") && f.starts_with("rotate") {
                    continue;
                }
                g.case(&format!("{f}({a}, {c})"), "img");
            }
        }
        let a = g.operand("gv");
        g.case(&format!("- {a}"), "img");
        let a = g.operand("gv");
        g.case(&format!("abs {a}"), "img");
        // to_integer: operands that fit the result, as the body would raise a range error.
        let (s, b) = g.vec();
        let s: String = s.chars().take(31).collect();
        g.case(&format!("to_integer(uv(\"{s}\", {b}))"), "img");
        g.case(&format!("to_integer(gv(\"{s}\", {b}))"), "img");
    }
    let mut out = String::from(HEAD);
    out.push_str(&g.out);
    out.push_str("    wait;\n  end process;\nend architecture;\n");
    out
}

fn run(dir: &Path, args: &[&str], native: bool) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_yavs"));
    c.args(args).current_dir(dir);
    // `YAVS_IEEE_TIER=llvm` (a build with the `llvm` feature) runs the same comparison on the
    // compiled tier.
    if let (Some(t), Some("run")) = (std::env::var_os("YAVS_IEEE_TIER"), args.first().copied()) {
        c.arg(format!("--tier={}", t.to_string_lossy()));
    }
    if !native {
        c.env("YAVS_NO_NATIVE_IEEE", "1");
    }
    c.output().expect("run yavs")
}

fn compare(seed: u64, std: &str) {
    let dir = tempdir("ieee");
    let src = testbench(seed, iters(25));
    std::fs::write(dir.join("t.vhd"), &src).expect("write");
    if let Some(keep) = std::env::var_os("YAVS_IEEE_KEEP") {
        let _ = std::fs::write(Path::new(&keep).join(format!("t{seed}.vhd")), &src);
    }
    let std_arg = format!("--std={std}");
    let a = run(
        &dir,
        &["analyze", &std_arg, "--work=work:lib", "t.vhd"],
        true,
    );
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    let go = |native| run(&dir, &["run", &std_arg, "--work=work:lib", "t"], native);
    let (n, v) = (go(true), go(false));
    let (ns, vs) = (
        String::from_utf8_lossy(&n.stdout),
        String::from_utf8_lossy(&v.stdout),
    );
    assert!(
        ns.lines().count() > 1000 / scale(),
        "too few results:\n{}",
        String::from_utf8_lossy(&n.stderr)
    );
    for (i, (x, y)) in ns.lines().zip(vs.lines()).enumerate() {
        assert_eq!(x, y, "line {i} (seed {seed}, std {std})");
    }
    assert_eq!(ns.lines().count(), vs.lines().count());
    assert_eq!(n.stderr, v.stderr);
    assert_eq!(n.status.code(), v.status.code());
}

#[test]
fn native_matches_bodies_2008() {
    for seed in 1..=8 {
        compare(seed, "2008");
    }
}

#[test]
fn native_matches_bodies_2019() {
    compare(11, "2019");
}

const ERRORS: &str = r#"library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity e is generic (k : integer := 0); end entity;
architecture a of e is
begin
  process
    variable u : unsigned(31 downto 0) := x"80000000";
    variable s : signed(39 downto 0) := x"8000000000";
    variable n : integer := -1;
    variable z : unsigned(0 downto 1);
    variable r : unsigned(7 downto 0);
    variable v : std_logic_vector(3 downto 0);
    variable i : integer;
  begin
    case k is
      when 0 => i := to_integer(u);
      when 1 => i := to_integer(s);
      when 2 => r := to_unsigned(n, 8);
      when 3 => r := r + n;
      when 4 => r := resize(r, n);
      when 5 => z := rotate_left(z, 1);
      when 6 => v := "0101" and "010";
      when 7 => if r < n then null; end if;
      when 8 => r := shift_left(r, n);
      when 9 => r := x"00"; r := r / r;
      when 10 => r := x"07"; r := r mod (r - r);
      when 11 => r := x"07"; r := r rem (r - r);
      when 12 => r := r xor unsigned'("010");
      when 13 => r := r nand unsigned'("010");
      when others => null;
    end case;
    report "end";
    wait;
  end process;
end architecture;
"#;

/// Runtime errors and failures: same exit code and message. The VHDL stack of an error raised
/// inside a body (`to_integer` overflow) has no frame for the native, so only the first line
/// is compared.
#[test]
fn native_errors_match_bodies() {
    let dir = tempdir("ieee");
    std::fs::write(dir.join("e.vhd"), ERRORS).expect("write");
    let a = run(
        &dir,
        &["analyze", "--std=2008", "--work=work:lib", "e.vhd"],
        true,
    );
    assert!(a.status.success());
    for k in 0..14 {
        let g = format!("-gk={k}");
        let go = |native| {
            run(
                &dir,
                &["run", "--std=2008", "--work=work:lib", "e", &g],
                native,
            )
        };
        let (n, v) = (go(true), go(false));
        assert_eq!(n.status.code(), v.status.code(), "k={k}");
        assert_eq!(n.stdout, v.stdout, "k={k}");
        let first = |o: &Output| {
            String::from_utf8_lossy(&o.stderr)
                .lines()
                .next()
                .map(str::to_string)
        };
        assert_eq!(first(&n), first(&v), "k={k}");
    }
}

fn edges() -> String {
    let mut rng = Rng(7);
    let mut drive = String::new();
    for _ in 0..400 {
        let c = b"UX01ZWLH-"[rng.below(9) as usize] as char;
        let _ = writeln!(drive, "    c <= '{c}'; wait for 1 ns;");
    }
    format!(
        r#"library ieee;
use ieee.std_logic_1164.all;
entity ed is end entity;
architecture a of ed is
  signal c : std_logic;
begin
  process begin
{drive}    wait;
  end process;
  process (c) begin
    if rising_edge(c) then report "rise " & std_logic'image(c); end if;
    if falling_edge(c) then report "fall " & std_logic'image(c); end if;
  end process;
end architecture;
"#
    )
}

/// `rising_edge` and `falling_edge` of a signal that takes every `std_ulogic` value.
#[test]
fn native_edges_match_bodies() {
    let dir = tempdir("ieee");
    std::fs::write(dir.join("ed.vhd"), edges()).expect("write");
    let a = run(
        &dir,
        &["analyze", "--std=2008", "--work=work:lib", "ed.vhd"],
        true,
    );
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    let go = |native| {
        run(
            &dir,
            &["run", "--std=2008", "--work=work:lib", "ed"],
            native,
        )
    };
    let (n, v) = (go(true), go(false));
    assert_eq!(n.stdout, v.stdout);
    assert!(String::from_utf8_lossy(&n.stdout).contains("rise"));
    assert_eq!(n.status.code(), v.status.code());
}

/// `math_real`: the exact functions and every error report agree with the bodies; the
/// transcendental results agree to 1e-5 (the bodies are series, the natives use `f64`).
#[test]
fn native_math_matches_bodies() {
    let lit = |x: f64| {
        if x.abs() >= 1e15 {
            format!("real'({x:.1e})")
        } else {
            format!("real'({x:.4})")
        }
    };
    let mut body = String::new();
    let xs = [
        -3.5,
        -2.5,
        -1.0,
        -0.5,
        0.0,
        0.5,
        1.0,
        1.5,
        2.0,
        2.5,
        10.0,
        123.456,
        2147483646.5,
        3.0e9,
        1.0e300,
    ];
    for x in xs {
        let xs = lit(x);
        for f in ["floor", "ceil", "round", "trunc", "sign"] {
            let _ = writeln!(body, "    report \"{f} \" & real'image({f}({xs}));");
        }
        // Domain checks and their error values.
        for f in ["sqrt", "log", "log2", "log10"] {
            let _ = writeln!(
                body,
                "    report \"{f} \" & real'image(1.0 + 1.0e-9 * {f}({xs}) * 0.0);"
            );
        }
        let _ = writeln!(
            body,
            "    report \"close \" & boolean'image(abs(sqrt(abs({xs})) - sqrt(abs({xs}))) < 1.0e-9);"
        );
    }
    for (a, b) in [
        (2.0, 3.0),
        (-2.0, 0.0),
        (-2.0, 2.0),
        (0.0, -1.0),
        (0.0, 2.0),
        (4.0, 0.5),
        (1.0, 9.0),
    ] {
        let (la, lb) = (lit(a), lit(b));
        let _ = writeln!(
            body,
            "    report \"pow \" & real'image({la} ** {lb} * 0.0);"
        );
    }
    let _ = writeln!(
        body,
        "    report \"powi \" & real'image((integer'(-2) ** real'(2.0)) * 0.0);"
    );
    let _ = writeln!(body, "    report \"exp \" & real'image(exp(1000.0) * 0.0);");
    let _ = writeln!(body, "    report \"exp \" & real'image(exp(-1000.0));");
    let _ = writeln!(
        body,
        "    report \"logb \" & real'image(log(8.0, 2.0) * 0.0);"
    );
    let _ = writeln!(
        body,
        "    report \"logb \" & real'image(log(8.0, 1.0) * 0.0);"
    );
    let _ = writeln!(
        body,
        "    report \"logb \" & real'image(log(-8.0, 2.0) * 0.0);"
    );
    let _ = writeln!(
        body,
        "    report \"rel \" & boolean'image(abs(exp(2.5) - 12.182493960703473) < 1.0e-5 and abs(sin(1.0) - 0.8414709848078965) < 1.0e-5 and abs(cos(1.0) - 0.5403023058681398) < 1.0e-5 and abs(log(10.0) - 2.302585092994046) < 1.0e-5 and abs(sqrt(2.0) - 1.4142135623730951) < 1.0e-5);"
    );
    let src = format!(
        "library ieee;\nuse ieee.math_real.all;\nentity m is end entity;\narchitecture a of m is\nbegin\n  process begin\n{body}    wait;\n  end process;\nend architecture;\n"
    );
    let dir = tempdir("ieee");
    std::fs::write(dir.join("m.vhd"), src).expect("write");
    let a = run(
        &dir,
        &["analyze", "--std=2008", "--work=work:lib", "m.vhd"],
        true,
    );
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    let go = |native| run(&dir, &["run", "--std=2008", "--work=work:lib", "m"], native);
    let (n, v) = (go(true), go(false));
    let (ns, vs) = (
        String::from_utf8_lossy(&n.stdout),
        String::from_utf8_lossy(&v.stdout),
    );
    for (i, (x, y)) in ns.lines().zip(vs.lines()).enumerate() {
        assert_eq!(x, y, "line {i}");
    }
    assert_eq!(ns.lines().count(), vs.lines().count());
    assert!(ns.contains("rel true"));
    assert_eq!(n.stderr, v.stderr);
    assert_eq!(n.status.code(), v.status.code());
}

const ARITH_HEAD: &str = r#"library ieee;
use ieee.std_logic_1164.all;
use ieee.std_logic_arith.all;
entity ta is end entity;
architecture a of ta is
  function img (x : std_ulogic_vector) return string is
    variable r : string(1 to x'length);
    variable k : integer := 0;
  begin
    for i in x'range loop
      k := k + 1;
      r(k) := std_ulogic'image(x(i))(2);
    end loop;
    return integer'image(x'left) & "," & integer'image(x'right) & "," &
           boolean'image(x'ascending) & ":" & r;
  end function;
  function img (x : unsigned) return string is
  begin return img(std_ulogic_vector(x)); end function;
  function img (x : signed) return string is
  begin return img(std_ulogic_vector(x)); end function;
  function img (b : boolean) return string is
  begin return boolean'image(b); end function;
  function cv (c : character) return std_ulogic is
  begin
    case c is
      when 'U' => return 'U'; when 'X' => return 'X'; when '0' => return '0';
      when '1' => return '1'; when 'Z' => return 'Z'; when 'W' => return 'W';
      when 'L' => return 'L'; when 'H' => return 'H'; when others => return '-';
    end case;
  end function;
  function sv (s : string; l : integer; asc : boolean) return std_ulogic_vector is
    variable d : std_ulogic_vector(l downto l - s'length + 1);
    variable u : std_ulogic_vector(l to l + s'length - 1);
  begin
    if asc then
      for i in u'range loop u(i) := cv(s(s'left + i - l)); end loop;
      return u;
    else
      for i in d'range loop d(i) := cv(s(s'left + l - i)); end loop;
      return d;
    end if;
  end function;
  function uv (s : string; l : integer; asc : boolean) return unsigned is
  begin return unsigned(sv(s, l, asc)); end function;
  function gv (s : string; l : integer; asc : boolean) return signed is
  begin return signed(sv(s, l, asc)); end function;
begin
  process
  begin
"#;

/// The Synopsys `std_logic_arith` helpers against their bodies, metavalues included (the
/// natives hand those calls back to the body).
#[test]
fn native_synopsys_arith_matches_bodies() {
    let mut g = Gen {
        rng: Rng(99),
        out: String::new(),
        n: 0,
    };
    for _ in 0..iters(150) {
        for op in ["+", "-", "*"] {
            for (ta, tb) in [("uv", "uv"), ("gv", "gv"), ("uv", "gv"), ("gv", "uv")] {
                let (a, b) = (g.operand(ta), g.operand(tb));
                if a.contains("\"\"") || b.contains("\"\"") {
                    continue; // null operands raise index errors in the bodies
                }
                let r = if ta == "uv" && tb == "uv" {
                    "unsigned"
                } else {
                    "signed"
                };
                g.case(&format!("{r}'({a} {op} {b})"), "img");
            }
        }
        for op in ["<", "<=", ">", ">=", "=", "/="] {
            for (ta, tb) in [("uv", "uv"), ("gv", "gv"), ("uv", "gv"), ("gv", "uv")] {
                let (a, b) = (g.operand(ta), g.operand(tb));
                if a.contains("\"\"") || b.contains("\"\"") {
                    continue;
                }
                g.case(&format!("{a} {op} {b}"), "img");
            }
        }
        for ty in ["uv", "gv"] {
            let a = g.operand(ty);
            if a.contains("\"\"") {
                continue;
            }
            let s = 1 + g.rng.below(40);
            for (f, r) in [
                ("conv_unsigned", "unsigned"),
                ("conv_signed", "signed"),
                ("conv_std_logic_vector", "std_logic_vector"),
            ] {
                g.case(&format!("{r}'({f}({a}, {s}))"), "img");
            }
        }
        let i = g.int(false);
        let s = 1 + g.rng.below(40);
        for (f, r) in [
            ("conv_unsigned", "unsigned"),
            ("conv_signed", "signed"),
            ("conv_std_logic_vector", "std_logic_vector"),
        ] {
            g.case(&format!("{r}'({f}({i}, {s}))"), "img");
        }
    }
    let src = format!(
        "{ARITH_HEAD}{}    wait;\n  end process;\nend architecture;\n",
        g.out
    );
    let dir = tempdir("ieee");
    std::fs::write(dir.join("ta.vhd"), src).expect("write");
    let a = run(
        &dir,
        &["analyze", "--std=2008", "--work=work:lib", "ta.vhd"],
        true,
    );
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    let go = |native| {
        run(
            &dir,
            &["run", "--std=2008", "--work=work:lib", "ta"],
            native,
        )
    };
    let (n, v) = (go(true), go(false));
    let (ns, vs) = (
        String::from_utf8_lossy(&n.stdout),
        String::from_utf8_lossy(&v.stdout),
    );
    assert!(
        ns.lines().count() > 800 / scale(),
        "{}",
        String::from_utf8_lossy(&n.stderr)
    );
    for (i, (x, y)) in ns.lines().zip(vs.lines()).enumerate() {
        assert_eq!(x, y, "line {i}");
    }
    assert_eq!(ns.lines().count(), vs.lines().count());
    assert_eq!(n.stderr, v.stderr);
    assert_eq!(n.status.code(), v.status.code());
}

/// The head of the Synopsys operator tests: `uv`/`gv` build the package's own `unsigned` and
/// `signed`, `lv` a `std_logic_vector`; `extra` is the `use` clause of `std_logic_unsigned` or
/// `std_logic_signed` (or nothing).
fn syn_head(extra: &str) -> String {
    ARITH_HEAD
        .replace("entity ta is", "entity t is")
        .replace("of ta is", "of t is")
        .replacen(
            "use ieee.std_logic_arith.all;\n",
            &format!("use ieee.std_logic_arith.all;\n{extra}"),
            1,
        )
        .replacen(
            "begin\n  process\n",
            "  function lv (s : string; l : integer; asc : boolean) return std_logic_vector is\n  begin return std_logic_vector(sv(s, l, asc)); end function;\nbegin\n  process\n",
            1,
        )
}

/// The patterns of an `n` element operand: zeros, ones, alternating, only the top or the bottom
/// element set, and the alternating pattern in `L` and `H`.
fn syn_pats(n: usize) -> Vec<String> {
    let alt: String = (0..n).map(|i| if i % 2 == 0 { '1' } else { '0' }).collect();
    vec![
        "0".repeat(n),
        "1".repeat(n),
        alt.clone(),
        format!("1{}", "0".repeat(n - 1)),
        format!("{}1", "0".repeat(n - 1)),
        alt.replace('1', "H").replace('0', "L"),
        format!("0{}", "1".repeat(n - 1)),
    ]
}

/// Each of `exprs` as a `report` with its result type spelled out (the operators overload on it).
fn syn_report(body: &mut String, expr: &str, rty: &str) {
    let e = match rty {
        "b" => format!("img(boolean'({expr}))"),
        "i" => format!("integer'image({expr})"),
        "v" => format!("img(std_ulogic_vector(std_logic_vector'({expr})))"),
        t => format!("img({t}'({expr}))"),
    };
    let _ = writeln!(body, "    report {e};");
}

/// The operators of the Synopsys packages against their bodies: every pair of operand kinds,
/// lengths around the one-word limit, both directions with lower bounds other than zero, extreme
/// and out of range integers (the packages truncate them), weak bits, metavalues (the bodies
/// warn and give all `X`) and the results of both vector types. With `wide_int` (2019) only the
/// integers beyond 32 bits, which are the one thing the revisions change.
fn syn_oper_test(pkg: &str, std: &str, wide_int: bool) {
    let (ua, sa) = match pkg {
        "arith" => ("uv", "gv"),
        _ => ("lv", "lv"),
    };
    let extra = match pkg {
        "unsigned" => "use ieee.std_logic_unsigned.all;\n",
        "signed" => "use ieee.std_logic_signed.all;\n",
        _ => "",
    };
    // The vector type of each operand kind and the types an operator of that mix may return.
    let mut kinds: Vec<(&str, &str, Vec<&str>)> = Vec::new();
    if pkg == "arith" {
        kinds.push((ua, ua, vec!["unsigned", "v"]));
        kinds.push((sa, sa, vec!["signed", "v"]));
        kinds.push((ua, sa, vec!["signed", "v"]));
        kinds.push((sa, ua, vec!["signed", "v"]));
    } else {
        kinds.push(("lv", "lv", vec!["v"]));
    }
    let lens: [(usize, usize); 8] = [
        (1, 1),
        (1, 4),
        (9, 5),
        (32, 32),
        (33, 31),
        (63, 64),
        (64, 64),
        (65, 3),
    ];
    let ints: Vec<&str> = if wide_int {
        vec![
            "4294967296",
            "(-4294967297)",
            "9223372036854775807",
            "(-9223372036854775807 - 1)",
            "(-2147483647 - 1)",
        ]
    } else {
        vec![
            "0",
            "(-1)",
            "517",
            "(-518)",
            "2147483647",
            "(-2147483647 - 1)",
        ]
    };
    // VHDL-1993 hides no relational operator: `std_logic_vector` ones are ambiguous there.
    let cmps = !(std == "1993" && pkg != "arith");
    let mut body = String::new();
    let mut k = 0usize;
    let lit = |ty: &str, x: &str, n: usize, k: usize| {
        let asc = k % 2 == 1;
        // `sv` declares both directions, so the left bound is at least `n - 1` either way.
        let lo = [0usize, 3, 100][k % 3];
        let l = if asc { lo.max(n - 1) } else { lo + n - 1 };
        format!("{ty}(\"{x}\", {l}, {asc})")
    };
    if !wide_int {
        for (ta, tb, rtys) in &kinds {
            let mixed = ta != tb;
            for &(na, nb) in &lens {
                let (pa, pb) = (syn_pats(na), syn_pats(nb));
                for i in [2usize, 5, 3] {
                    let (a, b) = (&pa[i], &pb[(i * 3 + 1) % pb.len()]);
                    k += 1;
                    let (x, y) = (lit(ta, a, na, k), lit(tb, b, nb, k + 1));
                    for op in ["+", "-"] {
                        // The `std_logic_vector` result once per pair, the package's own type always.
                        for r in rtys.iter().take(if i == 2 { 2 } else { 1 }) {
                            syn_report(&mut body, &format!("{x} {op} {y}"), r);
                        }
                    }
                    if na + nb < 80 && !(mixed && na + nb > 70) {
                        for r in rtys.iter().take(if i == 2 { 2 } else { 1 }) {
                            syn_report(&mut body, &format!("{x} * {y}"), r);
                        }
                    }
                    for op in ["<", "<=", ">", ">=", "=", "/="] {
                        if cmps {
                            syn_report(&mut body, &format!("{x} {op} {y}"), "b");
                        }
                    }
                }
            }
        }
    }
    // An integer against a vector of each length and kind, both orders.
    let int_kinds: Vec<(&str, &str)> = if pkg == "arith" {
        vec![(ua, "unsigned"), (sa, "signed")]
    } else {
        vec![("lv", "v")]
    };
    for (ta, rt) in int_kinds {
        for n in [1usize, 2, 8, 32, 33, 63, 64, 65] {
            let pa = syn_pats(n);
            let pats: &[usize] = if n > 8 { &[2, 5] } else { &[2, 1, 3] };
            for &i in pats {
                for (j, int) in ints.iter().enumerate() {
                    k += 1;
                    let x = lit(ta, &pa[i], n, k);
                    for op in ["+", "-"] {
                        syn_report(&mut body, &format!("{x} {op} {int}"), rt);
                        syn_report(&mut body, &format!("{int} {op} {x}"), rt);
                        if (i + j) % 3 == 0 {
                            syn_report(&mut body, &format!("{x} {op} {int}"), "v");
                        }
                    }
                    for op in ["<", ">=", "="] {
                        if cmps {
                            syn_report(&mut body, &format!("{x} {op} {int}"), "b");
                            syn_report(&mut body, &format!("{int} {op} {x}"), "b");
                        }
                    }
                }
            }
        }
    }
    // `shl` and `shr`: counts below, at and beyond the length, a count of 31 elements, weak and
    // undefined elements in the count.
    if !wide_int {
        let counts = [
            "0",
            "1",
            "10",
            "111",
            "100000",
            "1000000",
            "1000001",
            "L",
            "H",
            "X",
            "H0L",
            "0000000000000000000000000000001",
        ];
        for (ta, rt) in [(ua, "unsigned"), (sa, "signed")] {
            if pkg != "arith" && ta == sa && rt == "signed" {
                continue;
            }
            for n in [1usize, 2, 8, 33, 64, 65] {
                let pa = syn_pats(n);
                for &i in &[2usize, 5, 3] {
                    for (j, c) in counts.iter().enumerate() {
                        k += 1;
                        let x = lit(ta, &pa[i], n, k);
                        let cnt = lit(if pkg == "arith" { "uv" } else { "lv" }, c, c.len(), k + 1);
                        let r = if pkg == "arith" { rt } else { "v" };
                        if (i + j) % 2 == 0 || n <= 8 {
                            syn_report(&mut body, &format!("shl({x}, {cnt})"), r);
                            syn_report(&mut body, &format!("shr({x}, {cnt})"), r);
                        }
                    }
                }
            }
        }
    }
    // A metavalue in the argument of a shift moves with the others.
    if !wide_int {
        for m in "UXZW-".chars() {
            for n in [3usize, 65] {
                let mut p: Vec<char> = syn_pats(n)[2].chars().collect();
                p[n / 2] = m;
                let a: String = p.into_iter().collect();
                for c in ["1", "10", "111", "L"] {
                    k += 1;
                    let x = lit(ua, &a, n, k);
                    let cnt = lit(if pkg == "arith" { "uv" } else { "lv" }, c, c.len(), k + 1);
                    let r = if pkg == "arith" { "unsigned" } else { "v" };
                    syn_report(&mut body, &format!("shl({x}, {cnt})"), r);
                    syn_report(&mut body, &format!("shr({x}, {cnt})"), r);
                }
                if pkg == "arith" {
                    let x = lit(sa, &a, n, k);
                    let cnt = lit("uv", "10", 2, k + 1);
                    syn_report(&mut body, &format!("shr({x}, {cnt})"), "signed");
                }
            }
        }
    }
    // Metavalues: each code at the first and the last element of either operand.
    if !wide_int {
        for (ta, tb, rtys) in &kinds {
            for (na, nb) in [(1usize, 1usize), (33, 33), (64, 64)] {
                let good = |n: usize| syn_pats(n)[2].clone();
                for m in "UXZW-".chars() {
                    for at in [0, na - 1] {
                        let mut s: Vec<char> = good(na).chars().collect();
                        s[at] = m;
                        let a: String = s.into_iter().collect();
                        k += 1;
                        let (x, y) = (lit(ta, &a, na, k), lit(tb, &good(nb), nb, k + 1));
                        for r in rtys.iter().take(1) {
                            syn_report(&mut body, &format!("{x} + {y}"), r);
                            syn_report(&mut body, &format!("{y} - {x}"), r);
                        }
                        if cmps {
                            syn_report(&mut body, &format!("{x} < {y}"), "b");
                            syn_report(&mut body, &format!("{y} = {x}"), "b");
                        }
                        let own = match *ta {
                            "uv" => "unsigned",
                            "gv" => "signed",
                            _ => "v",
                        };
                        syn_report(&mut body, &format!("{x} + 3"), own);
                        if cmps {
                            syn_report(&mut body, &format!("{x} >= 3"), "b");
                        }
                    }
                }
            }
        }
    }
    let src = format!(
        "{}{body}    wait;\n  end process;\nend architecture;\n",
        syn_head(extra)
    );
    // Every statement ran: an error would end both runs at the same place and hide the rest.
    let code = diff_run(&format!("syn_oper_{pkg}_{std}"), &src, "t", std, 150);
    assert_eq!(code, Some(0), "{pkg} {std}");
}

#[test]
fn native_synopsys_operators_match_bodies_arith() {
    for (std, wide) in [("2008", false), ("2019", true), ("1993", false)] {
        syn_oper_test("arith", std, wide);
    }
}

#[test]
fn native_synopsys_operators_match_bodies_unsigned() {
    for (std, wide) in [("2008", false), ("2019", true), ("1993", false)] {
        syn_oper_test("unsigned", std, wide);
    }
}

#[test]
fn native_synopsys_operators_match_bodies_signed() {
    for (std, wide) in [("2008", false), ("2019", true), ("1993", false)] {
        syn_oper_test("signed", std, wide);
    }
}

/// `conv_unsigned`, `conv_signed`, `conv_std_logic_vector`, `ext`, `sxt` and `conv_integer`
/// against their bodies: sizes around the one-word limit and below the vector's length, integers
/// that the packages truncate (`INTEGER'LOW` only for the signed ones: the unsigned body fails
/// on it), weak bits, metavalues, both directions with lower bounds other than zero.
fn syn_conv_test(std: &str, wide_int: bool) {
    let ints: Vec<&str> = if wide_int {
        vec!["4294967296", "(-4294967297)", "9223372036854775807"]
    } else {
        vec!["0", "(-1)", "517", "(-518)", "2147483647", "1000000"]
    };
    let mut body = String::new();
    let mut k = 0usize;
    let lit = |ty: &str, x: &str, n: usize, k: usize| {
        let asc = k % 2 == 1;
        let lo = [0usize, 3, 100][k % 3];
        let l = if asc { lo.max(n - 1) } else { lo + n - 1 };
        format!("{ty}(\"{x}\", {l}, {asc})")
    };
    let sizes = [1usize, 2, 7, 8, 31, 32, 33, 63, 64, 65, 70];
    for n in [1usize, 2, 8, 31, 32, 33, 63, 64, 65] {
        let pats = syn_pats(n);
        for i in [2usize, 5, 3] {
            for &s in &sizes {
                k += 1;
                for (ty, f, r) in [
                    ("uv", "conv_unsigned", "unsigned"),
                    ("gv", "conv_unsigned", "unsigned"),
                    ("uv", "conv_signed", "signed"),
                    ("gv", "conv_signed", "signed"),
                    ("uv", "conv_std_logic_vector", "v"),
                    ("gv", "conv_std_logic_vector", "v"),
                ] {
                    let x = lit(ty, &pats[i], n, k);
                    syn_report(&mut body, &format!("{f}({x}, {s})"), r);
                }
                let x = lit("lv", &pats[i], n, k);
                syn_report(&mut body, &format!("ext({x}, {s})"), "v");
                syn_report(&mut body, &format!("sxt({x}, {s})"), "v");
            }
        }
    }
    for int in &ints {
        for &s in &sizes {
            for (f, r) in [
                ("conv_unsigned", "unsigned"),
                ("conv_signed", "signed"),
                ("conv_std_logic_vector", "v"),
            ] {
                syn_report(&mut body, &format!("{f}({int}, {s})"), r);
            }
        }
    }
    // INTEGER'LOW: the signed conversions only.
    let low = if wide_int {
        "(-9223372036854775807 - 1)"
    } else {
        "(-2147483647 - 1)"
    };
    for &s in &sizes {
        syn_report(&mut body, &format!("conv_signed({low}, {s})"), "signed");
        syn_report(
            &mut body,
            &format!("conv_std_logic_vector({low}, {s})"),
            "v",
        );
    }
    // Sizes below 1 and metavalues, which the bodies word themselves.
    for m in "UXZW-".chars() {
        for n in [1usize, 8, 33] {
            let mut s: Vec<char> = syn_pats(n)[2].chars().collect();
            s[n / 2] = m;
            let a: String = s.into_iter().collect();
            k += 1;
            syn_report(
                &mut body,
                &format!("conv_unsigned({}, 9)", lit("uv", &a, n, k)),
                "unsigned",
            );
            syn_report(&mut body, &format!("sxt({}, 40)", lit("lv", &a, n, k)), "v");
            syn_report(
                &mut body,
                &format!("conv_integer({})", lit("uv", &a[..n.min(30)], n.min(30), k)),
                "i",
            );
        }
    }
    // conv_integer of unsigned and signed vectors up to the package's limits.
    for n in [1usize, 2, 8, 30, 31] {
        for i in [2usize, 1, 5, 6] {
            k += 1;
            let x = lit("uv", &syn_pats(n)[i], n, k);
            syn_report(&mut body, &format!("conv_integer({x})"), "i");
        }
    }
    for n in [1usize, 2, 8, 31, 32] {
        for i in [2usize, 1, 5, 6, 3] {
            k += 1;
            let x = lit("gv", &syn_pats(n)[i], n, k);
            syn_report(&mut body, &format!("conv_integer({x})"), "i");
        }
    }
    for (pk, ns) in [("unsigned", [1usize, 8, 31]), ("signed", [1, 8, 32])] {
        let mut b = String::new();
        for n in ns {
            for i in [2usize, 1, 5, 3] {
                k += 1;
                let x = lit("lv", &syn_pats(n)[i], n, k);
                syn_report(&mut b, &format!("conv_integer({x})"), "i");
            }
        }
        let extra = format!("use ieee.std_logic_{pk}.all;\n");
        let src = format!(
            "{}{b}    wait;\n  end process;\nend architecture;\n",
            syn_head(&extra)
        );
        let code = diff_run(&format!("syn_conv_integer_{pk}_{std}"), &src, "t", std, 10);
        assert_eq!(code, Some(0), "{pk} {std}");
    }
    let src = format!(
        "{}{body}    wait;\n  end process;\nend architecture;\n",
        syn_head("")
    );
    let code = diff_run(&format!("syn_conv_{std}"), &src, "t", std, 300);
    assert_eq!(code, Some(0), "conversions {std}");
}

#[test]
fn native_synopsys_conversions_match_bodies() {
    syn_conv_test("2008", false);
    syn_conv_test("2019", true);
    syn_conv_test("1993", false);
}

/// `conv_integer` of a vector the package refuses (unsigned over 31 elements, signed over 32) is
/// the body's FAILURE: one process per case, the report and the exit status are the body's.
#[test]
fn native_synopsys_conv_integer_limits_match_bodies() {
    let cases = [
        format!("conv_integer(uv(\"{}\", 31, false))", "1".repeat(32)),
        format!("conv_integer(gv(\"{}\", 32, false))", "1".repeat(33)),
        "conv_integer(uv(\"\", -1, false))".to_string(),
        "conv_integer(gv(\"\", -1, false))".to_string(),
        "conv_unsigned(5, 0)".to_string(),
    ];
    for (i, c) in cases.iter().enumerate() {
        let r = if c.starts_with("conv_unsigned") {
            format!("img(unsigned'({c}))")
        } else {
            format!("integer'image({c})")
        };
        let src = format!(
            "{}    report {r};\n    wait;\n  end process;\nend architecture;\n",
            syn_head("")
        );
        diff_run(&format!("syn_limit{i}"), &src, "t", "2008", 0);
    }
}

/// `+` and `-` with a `std_ulogic` operand and the unary `+`, `-` and `abs` of the Synopsys
/// packages against their bodies: every bit value (`L` and `H` count, a metavalue is the body's
/// warning and all `X`), lengths 1 to 65, both directions with lower bounds other than zero, the
/// most negative value, weak bits and metavalues in a vector `abs` returns as it is.
fn syn_bit_unary_test(pkg: &str, std: &str) {
    let extra = match pkg {
        "unsigned" => "use ieee.std_logic_unsigned.all;\n",
        "signed" => "use ieee.std_logic_signed.all;\n",
        _ => "",
    };
    let mut body = String::new();
    let mut k = 0usize;
    let lit = |ty: &str, x: &str, n: usize, k: usize| {
        let asc = k % 2 == 1;
        let lo = [0usize, 3, 100][k % 3];
        let l = if asc { lo.max(n - 1) } else { lo + n - 1 };
        format!("{ty}(\"{x}\", {l}, {asc})")
    };
    // The vector type of each kind and the types its operators return.
    let kinds: Vec<(&str, Vec<&str>)> = if pkg == "arith" {
        vec![("uv", vec!["unsigned", "v"]), ("gv", vec!["signed", "v"])]
    } else {
        vec![("lv", vec!["v"])]
    };
    for n in [1usize, 8, 33, 63, 64, 65] {
        let mut pats = syn_pats(n);
        // A non-negative vector with weak bits, and one with a metavalue below its top element.
        pats.push(format!("L{}", "H".repeat(n - 1)));
        if n > 2 {
            pats.push(format!("0{}X1", "1".repeat(n - 3)));
            pats.push(format!("1{}W0", "0".repeat(n - 3)));
        }
        for (i, p) in pats.iter().enumerate() {
            for (ta, rtys) in &kinds {
                k += 1;
                let x = lit(ta, p, n, k);
                // Every bit value up to 8 elements, a strong or weak one and a metavalue above.
                for (j, b) in "01LHUXZW-".chars().enumerate() {
                    if n > 8 && j != i % 4 && j != 4 + i % 5 {
                        continue;
                    }
                    let b = format!("std_ulogic'('{b}')");
                    for e in [
                        format!("{x} + {b}"),
                        format!("{b} - {x}"),
                        format!("{b} + {x}"),
                        format!("{x} - {b}"),
                    ] {
                        syn_report(&mut body, &e, rtys[0]);
                    }
                    if let [_, r] = rtys.as_slice() {
                        syn_report(&mut body, &format!("{b} - {x}"), r);
                    }
                }
                let unary: &[&str] = match (pkg, *ta) {
                    ("arith", "uv") | ("unsigned", _) => &["+"],
                    _ => &["+", "-", "abs "],
                };
                for op in unary {
                    for r in rtys {
                        syn_report(&mut body, &format!("{op}{x}"), r);
                    }
                }
            }
        }
    }
    let src = format!(
        "{}{body}    wait;\n  end process;\nend architecture;\n",
        syn_head(extra)
    );
    let code = diff_run(&format!("syn_bit_{pkg}_{std}"), &src, "t", std, 300);
    assert_eq!(code, Some(0), "{pkg} {std}");
}

#[test]
fn native_synopsys_bit_and_unary_operators_match_bodies() {
    for std in ["2008", "1993"] {
        for pkg in ["arith", "unsigned", "signed"] {
            syn_bit_unary_test(pkg, std);
        }
    }
}

/// The new forms on a null vector: one process per case, the body's outcome.
#[test]
fn native_synopsys_bit_and_unary_null_match_bodies() {
    let cases = [
        ("", "img(unsigned'(uv(\"\", -1, false) + std_ulogic'('1')))"),
        ("", "img(signed'(std_ulogic'('H') - gv(\"\", -1, true)))"),
        ("", "img(signed'(- gv(\"\", -1, false)))"),
        ("", "img(signed'(abs gv(\"\", -1, false)))"),
        ("", "img(unsigned'(+ uv(\"\", -1, false)))"),
        (
            "use ieee.std_logic_signed.all;\n",
            "img(std_ulogic_vector(std_logic_vector'(abs lv(\"\", -1, true))))",
        ),
        (
            "use ieee.std_logic_unsigned.all;\n",
            "img(std_ulogic_vector(std_logic_vector'(lv(\"\", -1, true) - std_ulogic'('1'))))",
        ),
    ];
    for (i, (extra, c)) in cases.iter().enumerate() {
        let src = format!(
            "{}    report {c};\n    wait;\n  end process;\nend architecture;\n",
            syn_head(extra)
        );
        diff_run(&format!("syn_bit_null{i}"), &src, "t", "2008", 0);
    }
}

/// A null operand is the body's index error: one process per case so each error ends its own run.
#[test]
fn native_synopsys_operators_null_match_bodies() {
    let cases = [
        "unsigned'(uv(\"\", -1, false) + uv(\"101\", 2, false))",
        "unsigned'(uv(\"101\", 2, false) - uv(\"\", -1, false))",
        "uv(\"\", -1, false) < uv(\"1\", 0, false)",
        "gv(\"1\", 0, false) = gv(\"\", -1, true)",
        "signed'(gv(\"\", -1, true) + 3)",
        "uv(\"\", -1, false) > 3",
        "unsigned'(uv(\"\", -1, false) * uv(\"1\", 0, false))",
        // A count of 32 elements with its top element set: the body's 2 ** 31 overflows.
        "shl(uv(\"101\", 2, false), uv(\"10000000000000000000000000000000\", 31, false))",
        "shr(gv(\"101\", 2, false), uv(\"10000000000000000000000000000000\", 31, false))",
        "shl(gv(\"\", -1, false), uv(\"1\", 0, false))",
    ];
    for (i, c) in cases.iter().enumerate() {
        let src = format!(
            "{}    report img({c});\n    wait;\n  end process;\nend architecture;\n",
            syn_head("")
        );
        diff_run(&format!("syn_null{i}"), &src, "t", "2008", 0);
    }
}

/// Analyze `src` (entity `ent`) and run it with natives on and off: same stdout, stderr and
/// exit status, and at least `min` lines of output.
fn diff_run(tag: &str, src: &str, ent: &str, std: &str, min: usize) -> Option<i32> {
    let dir = tempdir("ieee");
    std::fs::write(dir.join("t.vhd"), src).expect("write");
    if let Some(keep) = std::env::var_os("YAVS_IEEE_KEEP") {
        let _ = std::fs::write(Path::new(&keep).join(format!("{tag}.vhd")), src);
    }
    let std_arg = format!("--std={std}");
    let a = run(
        &dir,
        &["analyze", &std_arg, "--work=work:lib", "t.vhd"],
        true,
    );
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    let go = |native| run(&dir, &["run", &std_arg, "--work=work:lib", ent], native);
    let (n, v) = (go(true), go(false));
    let (ns, vs) = (
        String::from_utf8_lossy(&n.stdout),
        String::from_utf8_lossy(&v.stdout),
    );
    assert!(
        ns.lines().count() >= min,
        "too few results:\n{}",
        String::from_utf8_lossy(&n.stderr)
    );
    for (i, (x, y)) in ns.lines().zip(vs.lines()).enumerate() {
        assert_eq!(x, y, "{tag} line {i} (std {std})");
    }
    assert_eq!(ns.lines().count(), vs.lines().count(), "{tag}");
    assert_eq!(n.stderr, v.stderr, "{tag}");
    assert_eq!(n.status.code(), v.status.code(), "{tag}");
    n.status.code()
}

/// `*` around the limit of its fast path (products of 64 and 65 elements) and off it: extreme
/// values, weak bits, metavalues, null and ascending operands, and slices that start inside a word.
#[test]
fn native_mul_limits_match_bodies() {
    let pats = |n: usize| -> Vec<String> {
        let alt: String = (0..n).map(|i| if i % 2 == 0 { '1' } else { '0' }).collect();
        vec![
            "0".repeat(n),
            "1".repeat(n),
            format!("1{}", "0".repeat(n - 1)),
            format!("{}1", "0".repeat(n - 1)),
            alt.clone(),
            alt.replace('1', "H").replace('0', "L"),
            format!("X{}", &alt[1..]),
        ]
    };
    let mut body = String::new();
    for (ty, f) in [("unsigned", "uv"), ("signed", "gv")] {
        for (a, b) in [
            (1, 1),
            (1, 63),
            (63, 1),
            (32, 32),
            (33, 31),
            (1, 64),
            (64, 1),
            (32, 33),
            (40, 40),
        ] {
            for x in pats(a) {
                for y in pats(b) {
                    let _ = writeln!(
                        body,
                        "    report img({f}(\"{x}\", {}, true) * {f}(\"{y}\", {}, false));",
                        a - 1,
                        b - 1
                    );
                }
            }
        }
        // Non-zero lower bounds, both directions for each operand.
        for (a, b) in [(3, 5), (32, 32), (33, 31), (1, 64), (64, 1)] {
            for x in pats(a) {
                for y in pats(b) {
                    for (da, db) in [("false", "true"), ("true", "false")] {
                        let _ = writeln!(
                            body,
                            "    report img({f}(\"{x}\", {}, {da}) * {f}(\"{y}\", {}, {db}));",
                            a + 6,
                            b + 2
                        );
                    }
                }
            }
        }
        let _ = writeln!(
            body,
            "    report img({f}(\"\", -1, true) * {f}(\"101\", 2, true));"
        );
        let _ = writeln!(
            body,
            "    report img({f}(\"101\", 2, true) * {f}(\"\", -1, true));"
        );
        let wide: String = (0..72)
            .map(|i| if i % 3 == 0 { '1' } else { '0' })
            .collect();
        for (hi, lo, hj, lj) in [
            (40, 9, 70, 39),
            (70, 40, 5, 4),
            (66, 33, 63, 34),
            (63, 0, 0, 0),
        ] {
            let _ = writeln!(
                body,
                "    report img({ty}'({f}(\"{wide}\", 71, false)({hi} downto {lo})) * {ty}'({f}(\"{wide}\", 71, false)({hj} downto {lj})));"
            );
        }
    }
    diff_run("mul_limits", &tb(&body), "t", "2008", 880);
}

/// `vector * integer` and `integer * vector`: the integer is converted at the vector's length,
/// so the lengths 1, 2, 31, 32, 33 and 64 (products of up to 64 elements and over), an integer that
/// fits, just fits and just does not (the body's truncation warning), zero, the most negative
/// value, weak bits, metavalues, null, both directions with a lower bound other than zero.
fn mul_integer_body(wide_int: bool, lens: &[usize], bit: bool) -> String {
    let mut body = String::new();
    for (ty, f, signed) in [("unsigned", "uv", false), ("signed", "gv", true)] {
        let _ = ty;
        for &n in lens {
            let alt: String = (0..n).map(|i| if i % 2 == 0 { '1' } else { '0' }).collect();
            let pats = [
                "0".repeat(n),
                "1".repeat(n),
                format!("1{}", "0".repeat(n - 1)),
                alt.clone(),
                alt.replace('1', "H").replace('0', "L"),
                format!("X{}", &alt[1..]),
                format!("{}U", &alt[..n - 1]),
            ];
            let top: i128 = if wide_int { 1 << 63 } else { 1 << 31 };
            let mut ints: Vec<i128> = vec![0, 1, 2, 3, 255, 65535, top - 1];
            if signed {
                ints.extend([-1, -2, -255, -top]);
            }
            for k in [n as u32, (n as u32).saturating_sub(1)] {
                if (1..64).contains(&k) {
                    let w: i128 = 1i128 << k;
                    // Unsigned: 2^n - 1 fits, 2^n does not; signed: 2^(n-1) - 1 fits, 2^(n-1) not.
                    let w = if signed { w >> 1 } else { w };
                    ints.extend([w - 1, w, w + 1]);
                    if signed {
                        ints.extend([-w, -w - 1, -w + 1]);
                    }
                }
            }
            if wide_int && n < 64 {
                ints.extend([1 << 40, top - 2]);
            }
            ints.retain(|v| *v < top && *v >= if signed { -top } else { 0 });
            ints.sort();
            ints.dedup();
            let lit = |v: i128| {
                if v == -(1i128 << 63) {
                    "(-9223372036854775807 - 1)".to_string()
                } else if v == -(1i128 << 31) && !wide_int {
                    "(-2147483647 - 1)".to_string()
                } else if v < 0 {
                    format!("(-{})", -v)
                } else {
                    v.to_string()
                }
            };
            // `bit` has no weak bits and no metavalues.
            for x in pats.iter().take(if bit { 4 } else { pats.len() }) {
                for (l, asc) in [(n - 1, false), (n + 6, true)] {
                    let vec = format!("{f}(\"{x}\", {l}, {asc})");
                    for v in &ints {
                        let _ = writeln!(body, "    report img({vec} * {});", lit(*v));
                        let _ = writeln!(body, "    report img({} * {vec});", lit(*v));
                    }
                }
            }
        }
        let _ = writeln!(body, "    report img({f}(\"\", -1, true) * 5);");
        let _ = writeln!(body, "    report img(5 * {f}(\"\", -1, false));");
        let _ = writeln!(body, "    report img({f}(\"\", 4, true) * 0);");
        if signed {
            let _ = writeln!(body, "    report img({f}(\"\", -1, true) * (-5));");
            let _ = writeln!(body, "    report img((-5) * {f}(\"\", -1, false));");
        }
    }
    body
}

#[test]
fn native_mul_integer_forms_match_bodies() {
    // 2008 has the full lengths; 2019 (64-bit integers) and 1993 (same bodies) fewer, to stay short.
    for (std, lens) in [
        ("2008", vec![1, 2, 31, 32, 33, 64]),
        ("2019", vec![1, 32, 33, 64]),
        ("1993", vec![2, 31, 33]),
    ] {
        diff_run(
            "mul_integer",
            &tb(&mul_integer_body(std == "2019", &lens, false)),
            "t",
            std,
            1000,
        );
    }
}

/// The same for `numeric_bit`, whose warning text names its own package.
#[test]
fn native_mul_integer_numeric_bit_matches_bodies() {
    let src = format!(
        "{BIT_HEAD}{}    wait;\n  end process;\nend architecture;\n",
        mul_integer_body(false, &[1, 2, 31, 32, 33], true)
    );
    diff_run("mul_integer_bit", &src, "t", "2008", 500);
}

/// A negative `NATURAL` operand is the body's subtype error, also when the vector is null; each
/// case is its own run since the error ends it.
#[test]
fn native_mul_negative_natural_matches_bodies() {
    for stmt in [
        "img(uv(\"101\", 2, true) * (-1))",
        "img((-1) * uv(\"101\", 2, true))",
        "img(uv(\"\", -1, true) * (-1))",
        "img((-3) * uv(\"\", -1, false))",
        "img(uv(\"X1\", 1, true) * (-2147483647 - 1))",
    ] {
        let body = format!("    report \"before\";\n    report {stmt};\n    report \"after\";\n");
        diff_run("mul_natural", &tb(&body), "t", "2008", 1);
    }
}

/// Operand patterns of `n` elements: extremes, alternating, weak bits, a metavalue in the middle
/// and at the end (`full`), else only the extremes and the alternating one.
fn wide_pats(n: usize, full: bool, bit: bool) -> Vec<String> {
    let alt: String = (0..n).map(|i| if i % 2 == 0 { '1' } else { '0' }).collect();
    let mut v = vec![
        "1".repeat(n),
        format!("1{}", "0".repeat(n - 1)),
        alt.clone(),
    ];
    if full {
        v.extend(["0".repeat(n), format!("0{}", "1".repeat(n - 1))]);
        if !bit {
            v.push(alt.replace('1', "H").replace('0', "L"));
            v.push(format!("{}X{}", &alt[..n / 2], &alt[n / 2 + 1..]));
        }
    }
    v
}

/// `*` of two vectors with 65 to 1025 result elements (the limit of the word-wise path is 1024):
/// products just over 64, equal and unequal lengths, extremes, weak bits, metavalues, both
/// directions with other index ranges. `full` is the whole set of patterns and index ranges,
/// else a few (the bodies are slow on long vectors).
fn wide_mul_body(pairs: &[(usize, usize)], full: bool, bit: bool) -> String {
    let mut body = String::new();
    for f in ["uv", "gv"] {
        for &(a, b) in pairs {
            for x in wide_pats(a, full, bit) {
                for y in wide_pats(b, full, bit) {
                    for (la, da, lb, db) in [
                        (a - 1, false, b - 1, false),
                        (a + 6, true, b + 2, false),
                        (a - 1, false, b + 5, true),
                    ]
                    .iter()
                    .take(if full { 2 } else { 1 })
                    {
                        let _ = writeln!(
                            body,
                            "    report img({f}(\"{x}\", {la}, {da}) * {f}(\"{y}\", {lb}, {db}));"
                        );
                    }
                }
            }
        }
    }
    body
}

const WIDE_PAIRS: [(usize, usize); 12] = [
    (33, 32),
    (33, 33),
    (1, 64),
    (64, 1),
    (64, 2),
    (40, 26),
    (64, 64),
    (100, 28),
    (65, 1),
    (3, 100),
    (130, 62),
    (200, 200),
];

#[test]
fn native_wide_mul_matches_bodies() {
    // Up to 1024 result elements, then 1025 (the general path).
    let big = [(512, 512), (513, 512), (1000, 24), (1, 1024), (2, 1025)];
    for (std, n, with_big) in [
        ("2008", WIDE_PAIRS.len(), true),
        ("2019", 8, false),
        ("1993", 4, false),
    ] {
        let mut body = wide_mul_body(&WIDE_PAIRS[..n], true, false);
        if with_big {
            body.push_str(&wide_mul_body(&big, false, false));
        }
        let ones = "1".repeat(99);
        let _ = writeln!(
            body,
            "    report img(uv(\"\", -1, true) * uv(\"{ones}\", 99, false));"
        );
        let _ = writeln!(
            body,
            "    report img(gv(\"{ones}\", 99, false) * gv(\"\", -1, true));"
        );
        diff_run("wide_mul", &tb(&body), "t", std, 500);
    }
}

#[test]
fn native_wide_mul_numeric_bit_matches_bodies() {
    let mut body = wide_mul_body(&WIDE_PAIRS[..10], true, true);
    body.push_str(&wide_mul_body(&[(513, 512)], false, true));
    let src = format!(
        "{BIT_HEAD}{}    wait;\n  end process;\nend architecture;\n",
        body
    );
    diff_run("wide_mul_bit", &src, "t", "2008", 500);
}

/// `vector * integer` and `integer * vector` where the vector (so the integer, converted at its
/// length) makes the product longer than 64 elements, with the integers that fit, just fit and
/// do not (the truncation warning) for vectors of 33 to 64 elements.
#[test]
fn native_wide_mul_integer_forms_match_bodies() {
    for (std, lens) in [
        ("2008", vec![33, 34, 40, 63, 64, 65, 66, 100, 128]),
        ("2019", vec![33, 64, 65, 129]),
        ("1993", vec![33, 65]),
    ] {
        diff_run(
            "wide_mul_integer",
            &tb(&mul_integer_body(std == "2019", &lens, false)),
            "t",
            std,
            1000,
        );
    }
    let src = format!(
        "{BIT_HEAD}{}    wait;\n  end process;\nend architecture;\n",
        mul_integer_body(false, &[33, 64, 65, 128], true)
    );
    diff_run("wide_mul_integer_bit", &src, "t", "2008", 500);
    // 512 and 513 elements (products of 1024 and 1026): a few operands, the bodies are slow.
    let mut body = String::new();
    for f in ["uv", "gv"] {
        for n in [512usize, 513] {
            for x in wide_pats(n, false, false) {
                for i in ["0", "1", "65535", "2147483647", "(-1)", "(-2147483647 - 1)"] {
                    if f == "uv" && i.starts_with('(') {
                        continue;
                    }
                    let _ = writeln!(
                        body,
                        "    report img({f}(\"{x}\", {}, false) * {i});",
                        n + 1
                    );
                    let _ = writeln!(body, "    report img({i} * {f}(\"{x}\", {}, true));", n + 1);
                }
            }
        }
    }
    diff_run("wide_mul_integer_big", &tb(&body), "t", "2008", 50);
}

/// Rotations of vectors of 65 to 1025 elements (the word-wise path takes up to 1024) by 0, 1, a
/// word, `n - 1`, `n`, more than `n` and the largest NATURAL, `rol` and `ror` with negative counts,
/// weak bits and metavalues (the general path), other index ranges.
#[test]
fn native_wide_rotate_matches_bodies() {
    for (std, sizes) in [
        ("2008", vec![65usize, 66, 127, 128, 129, 192, 1024, 1025]),
        ("2019", vec![65, 129, 1025]),
        ("1993", vec![66, 128]),
    ] {
        let mut body = String::new();
        for f in ["uv", "gv"] {
            for &n in &sizes {
                let pats = wide_pats(n, n < 200, false);
                let pats = if n < 200 { pats } else { pats[..2].to_vec() };
                for (pi, x) in pats.iter().enumerate() {
                    // Both directions of the index range, alternating over the patterns.
                    let (l, asc) = if pi % 2 == 0 {
                        (n - 1, false)
                    } else {
                        (n + 6, true)
                    };
                    let v = format!("{f}(\"{x}\", {l}, {asc})");
                    let nn = n as u64;
                    let counts = [0, 1, 63, 64, 65, nn - 1, nn, nn + 1, 2 * nn + 3, 2147483647];
                    for c in counts {
                        for r in ["rotate_left", "rotate_right"] {
                            let _ = writeln!(body, "    report img({r}({v}, {c}));");
                        }
                        if std != "1993" {
                            for (r, sign) in [("rol", ""), ("ror", "-")] {
                                let _ = writeln!(body, "    report img({v} {r} ({sign}{c}));");
                            }
                        }
                    }
                }
            }
        }
        diff_run("wide_rotate", &tb(&body), "t", std, 300);
    }
}

/// `numeric_bit` rotations of wide vectors.
#[test]
fn native_wide_rotate_numeric_bit_matches_bodies() {
    let mut body = String::new();
    for f in ["uv", "gv"] {
        for n in [65usize, 128, 1025] {
            for x in wide_pats(n, n < 200, true) {
                let v = format!("{f}(\"{x}\", {}, false)", n + 3);
                for c in [0, 1, 64, n - 1, n, n + 7] {
                    let _ = writeln!(body, "    report img(rotate_left({v}, {c}));");
                    let _ = writeln!(body, "    report img(rotate_right({v}, {c}));");
                    let _ = writeln!(body, "    report img({v} rol ({c}));");
                    let _ = writeln!(body, "    report img({v} ror (-{c}));");
                }
            }
        }
    }
    let src = format!(
        "{BIT_HEAD}{}    wait;\n  end process;\nend architecture;\n",
        body
    );
    diff_run("wide_rotate_bit", &src, "t", "2008", 500);
}

/// A negative count is the NATURAL subtype error of `rotate_*` also for a wide vector; each case
/// is its own run since the error ends it.
#[test]
fn native_wide_rotate_negative_count_matches_bodies() {
    let ones = "1".repeat(100);
    for stmt in [
        format!("img(rotate_left(uv(\"{ones}\", 99, false), -1))"),
        format!("img(rotate_right(gv(\"{ones}\", 99, false), -70))"),
    ] {
        let body = format!("    report \"before\";\n    report {stmt};\n    report \"after\";\n");
        diff_run("wide_rotate_neg", &tb(&body), "t", "2008", 1);
    }
}

/// `to_01` around the limit of its fast path (1, 63 to 65, 70 and 128 elements), null operands,
/// ascending and descending bounds, slices that start inside a word, every metavalue at the
/// first, a middle and the last element, and every `xmap`.
#[test]
fn native_to_01_limits_match_bodies() {
    let pats = |n: usize| -> Vec<String> {
        let alt: String = (0..n).map(|i| if i % 2 == 0 { '1' } else { '0' }).collect();
        vec![
            "0".repeat(n),
            "1".repeat(n),
            alt.clone(),
            alt.replace('1', "H").replace('0', "L"),
        ]
    };
    let metas = |n: usize| -> Vec<String> {
        let mut v = Vec::new();
        for m in "UXZW-".chars() {
            for at in [0, n / 2, n - 1] {
                let mut s: Vec<char> = pats(n)[2].chars().collect();
                s[at] = m;
                v.push(s.into_iter().collect());
            }
        }
        v
    };
    let mut body = String::new();
    for n in [1, 2, 63, 64, 65, 70, 128] {
        for (l, asc) in [(n - 1, "false"), (n + 3, "true")] {
            for x in pats(n) {
                for ty in ["sv", "uv", "gv"] {
                    let _ = writeln!(body, "    report img(to_01({ty}(\"{x}\", {l}, {asc})));");
                }
                let _ = writeln!(body, "    report img(to_01(sv(\"{x}\", {l}, {asc}), '1'));");
            }
            for x in metas(n) {
                for xm in "UX01ZWLH-".chars() {
                    let _ = writeln!(
                        body,
                        "    report img(to_01(sv(\"{x}\", {l}, {asc}), '{xm}'));"
                    );
                }
                for ty in ["uv", "gv"] {
                    let _ = writeln!(
                        body,
                        "    report img(to_01({ty}(\"{x}\", {l}, {asc}), 'X'));"
                    );
                    let _ = writeln!(body, "    report img(to_01({ty}(\"{x}\", {l}, {asc})));");
                }
            }
        }
    }
    for asc in ["true", "false"] {
        let _ = writeln!(body, "    report img(to_01(sv(\"\", -1, {asc})));");
        let _ = writeln!(body, "    report img(to_01(sv(\"\", -1, {asc}), 'X'));");
        let _ = writeln!(body, "    report img(to_01(uv(\"\", -1, {asc})));");
        let _ = writeln!(body, "    report img(to_01(gv(\"\", -1, {asc}), 'U'));");
    }
    let wide: String = (0..72)
        .map(|i| if i % 3 == 0 { '1' } else { 'H' })
        .collect();
    for (hi, lo) in [
        (40, 9),
        (70, 7),
        (63, 0),
        (64, 0),
        (65, 1),
        (71, 6),
        (62, 62),
    ] {
        let _ = writeln!(
            body,
            "    report img(to_01(sv(\"{wide}\", 71, false)({hi} downto {lo})));"
        );
        let _ = writeln!(
            body,
            "    report img(to_01(sv(\"{wide}\", 71, false)({hi} downto {lo}), '1'));"
        );
    }
    diff_run("to_01_limits", &tb(&body), "t", "2008", 3500);
}

/// Unary `-` of a `signed` around the limit of its fast path (1, 63 to 65, 70 and 128 elements),
/// null operands, ascending and descending bounds with a lower bound other than zero, the most
/// negative value, all ones, weak bits, every metavalue at the first, a middle and the last
/// element, slices that start inside a word, and `abs` (its body negates through the native).
#[test]
fn native_neg_limits_match_bodies() {
    let pats = |n: usize| -> Vec<String> {
        let alt: String = (0..n).map(|i| if i % 2 == 0 { '1' } else { '0' }).collect();
        vec![
            "0".repeat(n),
            "1".repeat(n),
            format!("1{}", "0".repeat(n - 1)),
            format!("{}1", "0".repeat(n - 1)),
            format!("0{}", "1".repeat(n - 1)),
            alt.clone(),
            alt.replace('1', "H").replace('0', "L"),
        ]
    };
    let mut body = String::new();
    for n in [1, 2, 7, 8, 63, 64, 65, 70, 128] {
        for (l, asc) in [(n - 1, "false"), (n + 3, "true")] {
            for x in pats(n) {
                let _ = writeln!(body, "    report img(-gv(\"{x}\", {l}, {asc}));");
                let _ = writeln!(body, "    report img(-(-gv(\"{x}\", {l}, {asc})));");
                let _ = writeln!(body, "    report img(abs gv(\"{x}\", {l}, {asc}));");
            }
            for m in "UXZW-".chars() {
                for at in [0, n / 2, n - 1] {
                    let mut s: Vec<char> = pats(n)[5].chars().collect();
                    s[at] = m;
                    let x: String = s.into_iter().collect();
                    let _ = writeln!(body, "    report img(-gv(\"{x}\", {l}, {asc}));");
                }
            }
        }
    }
    for asc in ["true", "false"] {
        let _ = writeln!(body, "    report img(-gv(\"\", -1, {asc}));");
    }
    let wide: String = (0..72)
        .map(|i| if i % 3 == 0 { '1' } else { 'H' })
        .collect();
    for (hi, lo) in [
        (40, 9),
        (70, 7),
        (63, 0),
        (64, 0),
        (65, 1),
        (71, 6),
        (62, 62),
    ] {
        let _ = writeln!(
            body,
            "    report img(-gv(\"{wide}\", 71, false)({hi} downto {lo}));"
        );
    }
    for std in ["2008", "2019"] {
        diff_run("neg_limits", &tb(&body), "t", std, 400);
    }
}

/// `abs` of a `signed` around the limit of its fast path (1, 2, 63 to 65, 70 and 128 elements):
/// null operands, both directions with a lower bound other than zero, non-negative and negative
/// operands of every pattern, the most negative value, weak bits, every metavalue at the first, a
/// middle and the last element, and slices that start inside a word.
#[test]
fn native_abs_limits_match_bodies() {
    let pats = |n: usize| -> Vec<String> {
        let alt: String = (0..n).map(|i| if i % 2 == 0 { '1' } else { '0' }).collect();
        vec![
            "0".repeat(n),
            "1".repeat(n),
            format!("1{}", "0".repeat(n - 1)),
            format!("{}1", "0".repeat(n - 1)),
            format!("0{}", "1".repeat(n - 1)),
            alt.clone(),
            alt.replace('1', "H").replace('0', "L"),
            format!("H{}", "L".repeat(n - 1)),
            format!("L{}", "H".repeat(n - 1)),
        ]
    };
    let mut body = String::new();
    for n in [1, 2, 7, 8, 63, 64, 65, 70, 128] {
        for (l, asc) in [(n - 1, "false"), (n + 3, "true")] {
            for x in pats(n) {
                let _ = writeln!(body, "    report img(abs gv(\"{x}\", {l}, {asc}));");
                let _ = writeln!(body, "    report img(abs (abs gv(\"{x}\", {l}, {asc})));");
            }
            for m in "UXZW-".chars() {
                for at in [0, n / 2, n - 1] {
                    for base in [5, 6] {
                        let mut s: Vec<char> = pats(n)[base].chars().collect();
                        s[at] = m;
                        let x: String = s.into_iter().collect();
                        let _ = writeln!(body, "    report img(abs gv(\"{x}\", {l}, {asc}));");
                    }
                }
            }
        }
    }
    for asc in ["true", "false"] {
        let _ = writeln!(body, "    report img(abs gv(\"\", -1, {asc}));");
    }
    let wide: String = (0..72)
        .map(|i| if i % 3 == 0 { '1' } else { 'H' })
        .collect();
    for (hi, lo) in [
        (40, 9),
        (70, 7),
        (63, 0),
        (64, 0),
        (65, 1),
        (71, 6),
        (62, 62),
        (0, 0),
    ] {
        let _ = writeln!(
            body,
            "    report img(abs gv(\"{wide}\", 71, false)({hi} downto {lo}));"
        );
    }
    for std in ["2008", "2019"] {
        diff_run("abs_limits", &tb(&body), "t", std, 800);
    }
}

/// Every numeric_std operation of two vectors, and of a vector and an integer, on one pair of
/// operands. The body of signed `mod` reads the raw sign element of the dividend, so the sign
/// element is `0`, `1`, `L` or `H` in all the patterns.
const WEAK_PROC: &str = r#"  type ints_t is array (natural range <>) of integer;
  constant ints : ints_t := (-7, -1, 1, 2, 5, 100);
  procedure chk (sa, sb : string; asc : boolean) is
    constant ga : signed := gv(sa, sa'length + 4, asc);
    constant gb : signed := gv(sb, sb'length + 4, not asc);
    constant ua : unsigned := uv(sa, sa'length + 4, asc);
    constant ub : unsigned := uv(sb, sb'length + 4, not asc);
  begin
    report img(ga / gb) & img(ga rem gb) & img(ga mod gb);
    report img(ua / ub) & img(ua rem ub) & img(ua mod ub);
    report img(ga + gb) & img(ga - gb) & img(ga * gb);
    report img(ua + ub) & img(ua - ub) & img(ua * ub);
    report img(ga < gb) & img(ga <= gb) & img(ga = gb) & img(ga > gb) & img(ga >= gb) & img(ga /= gb);
    report img(ua < ub) & img(ua <= ub) & img(ua = ub) & img(ua > ub) & img(ua >= ub) & img(ua /= ub);
    report img(resize(ga, sb'length)) & img(resize(ga, sa'length + 3)) & img(resize(ua, sb'length));
    report img(-ga) & img(abs ga);
    for k in ints'range loop
      report img(ga / ints(k)) & img(ga rem ints(k)) & img(ga mod ints(k));
      report img(ints(k) / gb) & img(ints(k) rem gb) & img(ints(k) mod gb);
      report img(ua / abs ints(k)) & img(ua rem abs ints(k)) & img(ua mod abs ints(k));
      report img(abs ints(k) / ub) & img(abs ints(k) mod ub);
      -- The body of `natural rem unsigned` lacks a RESIZE: its length error is not the natives'.
      if ub'length > 30 or abs ints(k) < 2 ** ub'length then
        report img(abs ints(k) rem ub);
      end if;
      report img(ga + ints(k)) & img(ints(k) - gb) & img(ga * ints(k)) & img(ints(k) * gb);
      report img(ga < ints(k)) & img(ints(k) >= gb) & img(ga = ints(k)) & img(ints(k) /= gb);
    end loop;
  end procedure;
"#;

/// Every operation on dividends and divisors whose sign element is `0`, `1`, `L` or `H`, the
/// dividend narrower, wider and as wide as the divisor, across a plane word.
#[test]
fn native_weak_sign_matches_bodies() {
    let rests = |n: usize| -> Vec<String> {
        let alt: String = (0..n).map(|i| if i % 2 == 0 { '1' } else { '0' }).collect();
        let mut v = vec![
            "0".repeat(n),
            "1".repeat(n),
            alt.clone(),
            alt.replace('1', "H").replace('0', "L"),
            "H".repeat(n),
            "L".repeat(n),
        ];
        v.dedup();
        v
    };
    let pats = |n: usize| -> Vec<String> {
        let mut v: Vec<String> = ['0', '1', 'L', 'H']
            .iter()
            .flat_map(|s| rests(n - 1).into_iter().map(move |r| format!("{s}{r}")))
            .collect();
        v.sort();
        v.dedup();
        v
    };
    let mut body = String::new();
    let mut k = 0;
    for (na, nb) in [
        (1, 1),
        (2, 2),
        (3, 3),
        (3, 4),
        (4, 3),
        (3, 50),
        (50, 3),
        (5, 2),
        (2, 5),
        (33, 3),
        (3, 65),
        (65, 2),
    ] {
        for a in pats(na) {
            for b in pats(nb) {
                // A zero divisor is the body's error.
                if !b.contains(['1', 'H']) {
                    continue;
                }
                k += 1;
                let _ = writeln!(body, "    chk(\"{a}\", \"{b}\", {});", k % 2 == 0);
            }
        }
    }
    let head = HEAD.replacen(
        "begin\n  process\n",
        &format!("{WEAK_PROC}begin\n  process\n"),
        1,
    );
    for std in ["2008", "2019", "1993"] {
        let src = format!("{head}{body}    wait;\n  end process;\nend architecture;\n");
        diff_run("weak_sign", &src, "t", std, 20000);
    }
}

/// Operand patterns of `n` elements for the matching operators: constants, `L`/`H`, one
/// non-binary element at the first, a middle and the last position, and `-` fillings.
fn match_pats(n: usize) -> Vec<String> {
    let alt: String = (0..n).map(|i| if i % 2 == 0 { '1' } else { '0' }).collect();
    let rep = |c: &str, k: usize| c.repeat(k);
    let mut v = vec![
        rep("0", n),
        rep("1", n),
        format!("1{}", rep("0", n - 1)),
        format!("{}1", rep("0", n - 1)),
        format!("0{}", rep("1", n - 1)),
        alt.clone(),
        alt.replace('1', "H").replace('0', "L"),
        format!("H{}", rep("L", n - 1)),
        format!("L{}", rep("H", n - 1)),
        rep("-", n),
        alt.replace('1', "-"),
    ];
    for m in "UXZW-".chars() {
        for at in [0, n / 2, n - 1] {
            let mut s: Vec<char> = alt.chars().collect();
            s[at] = m;
            v.push(s.into_iter().collect());
        }
    }
    v
}

/// Integer operands around the range of an `n`-element vector.
fn match_ints(n: usize, signed: bool, wide: bool) -> Vec<String> {
    let top: i128 = if wide { 1 << 63 } else { 1 << 31 };
    let mut v: Vec<i128> = vec![0, 1, 2, 255, top - 1];
    if signed {
        v.extend([-1, -2, -255, -top]);
    }
    for k in [n, n - 1] {
        if (1..63).contains(&k) {
            let w: i128 = if signed { 1 << (k - 1) } else { 1 << k };
            v.extend([w - 1, w, w + 1]);
            if signed {
                v.extend([-w, -w - 1, -w + 1]);
            }
        }
    }
    v.retain(|x| *x < top && *x >= if signed { -top } else { 0 });
    v.sort();
    v.dedup();
    v.iter()
        .map(|x| match *x {
            x if x == -top && wide => "(-9223372036854775807 - 1)".to_string(),
            x if x == -top => "(-2147483647 - 1)".to_string(),
            x if x < 0 => format!("(-{})", -x),
            x => x.to_string(),
        })
        .collect()
}

const QOPS: [&str; 6] = ["?<", "?<=", "?>", "?>=", "?=", "?/="];

/// The matching operators, `maximum`, `minimum` and `std_match` of vectors of equal and unequal
/// lengths around the limit of the one-word path, in both directions with a lower bound other
/// than zero.
fn match_vectors(lens: &[(usize, usize)], ops: bool) -> String {
    let mut body = String::new();
    for &(n, m) in lens {
        let (px, py) = (match_pats(n), match_pats(m));
        let np = px.len().min(py.len());
        // Every pattern for the short equal lengths, every fifth for the rest (test time).
        let stride = if n <= 8 && n == m { 1 } else { 5 };
        for i in (0..np).step_by(stride) {
            for k in [0, 7] {
                // The same pattern and a far one.
                let (x, y) = (&px[i], &py[(i + k) % np]);
                let f = if (i + k) % 2 == 0 { "uv" } else { "gv" };
                let dirs = if (i + k) % 3 == 0 {
                    [(n - 1, "false"), (m + 4, "true")]
                } else {
                    [(n + 5, "true"), (m - 1, "false")]
                };
                let a = format!("{f}(\"{x}\", {}, {})", dirs[0].0, dirs[0].1);
                let b = format!("{f}(\"{y}\", {}, {})", dirs[1].0, dirs[1].1);
                let _ = writeln!(body, "    report img(std_match({a}, {b}));");
                let sa = format!("std_logic_vector({a})");
                let sb = format!("std_logic_vector({b})");
                let _ = writeln!(body, "    report img(std_match({sa}, {sb}));");
                if ops {
                    for op in QOPS {
                        let _ = writeln!(body, "    report img({a} {op} {b});");
                    }
                    let _ = writeln!(body, "    report img(maximum({a}, {b}));");
                    let _ = writeln!(body, "    report img(minimum({a}, {b}));");
                }
            }
        }
    }
    for (f, e) in [("uv", "uv(\"\", -1, true)"), ("gv", "gv(\"\", -1, false)")] {
        let v = format!("{f}(\"101\", 2, true)");
        let _ = writeln!(body, "    report img(std_match({e}, {v}));");
        let _ = writeln!(body, "    report img(std_match({v}, {e}));");
        let _ = writeln!(body, "    report img(std_match({e}, {e}));");
        if ops {
            for op in QOPS {
                let _ = writeln!(body, "    report img({e} {op} {v});");
                let _ = writeln!(body, "    report img({v} {op} {e});");
                let _ = writeln!(body, "    report img({e} {op} {e});");
            }
            for g in ["maximum", "minimum"] {
                let _ = writeln!(body, "    report img({g}({e}, {v}));");
                let _ = writeln!(body, "    report img({g}({v}, {e}));");
                let _ = writeln!(body, "    report img({g}({e}, {e}));");
            }
        }
    }
    body
}

/// The integer forms of the matching operators, `maximum` and `minimum`: integers that fit,
/// just fit and just do not (the body's `to_unsigned` warning), null vectors, weak bits and
/// metavalues.
fn match_integers(lens: &[usize], wide: bool) -> String {
    let mut body = String::new();
    for (f, signed) in [("uv", false), ("gv", true)] {
        for &n in lens {
            let ints = match_ints(n, signed, wide);
            let pats = match_pats(n);
            for (j, x) in [0, 2, 5, 6, 14, 25].iter().map(|k| (*k, &pats[*k])) {
                let (l, asc) = if j % 2 == 0 {
                    (n - 1, "false")
                } else {
                    (n + 6, "true")
                };
                let v = format!("{f}(\"{x}\", {l}, {asc})");
                for (c, i) in ints.iter().enumerate() {
                    for op in QOPS {
                        if (c + j) % 2 == 0 {
                            let _ = writeln!(body, "    report img({v} {op} {i});");
                        } else {
                            let _ = writeln!(body, "    report img({i} {op} {v});");
                        }
                    }
                    let _ = writeln!(body, "    report img(maximum({v}, {i}));");
                    let _ = writeln!(body, "    report img(minimum({i}, {v}));");
                }
            }
        }
        for i in ["0", "5"] {
            let e = format!("{f}(\"\", -1, true)");
            for op in QOPS {
                let _ = writeln!(body, "    report img({e} {op} {i});");
                let _ = writeln!(body, "    report img({i} {op} {e});");
            }
            let _ = writeln!(body, "    report img(maximum({e}, {i}));");
            let _ = writeln!(body, "    report img(minimum({i}, {e}));");
        }
    }
    body
}

const MATCH_LENS: [(usize, usize); 14] = [
    (1, 1),
    (2, 2),
    (8, 8),
    (63, 63),
    (64, 64),
    (65, 65),
    (128, 128),
    (1, 2),
    (7, 8),
    (63, 64),
    (64, 63),
    (64, 65),
    (3, 70),
    (128, 63),
];

/// The matching operators, `maximum`, `minimum` and `std_match` of vectors against their bodies.
#[test]
fn native_match_minmax_vectors_match_bodies() {
    for std in ["2008", "2019"] {
        diff_run(
            "match_vectors",
            &tb(&match_vectors(&MATCH_LENS, true)),
            "t",
            std,
            3000,
        );
    }
    // 1993 has `std_match` only.
    diff_run(
        "match_vectors93",
        &tb(&match_vectors(&MATCH_LENS[..10], false)),
        "t",
        "1993",
        300,
    );
}

/// The integer forms around the range of the vector (2008 and 2019 integers).
#[test]
fn native_match_minmax_integers_match_bodies() {
    for (std, lens) in [
        ("2008", vec![1, 2, 32, 33, 64, 65]),
        ("2019", vec![1, 33, 64]),
    ] {
        diff_run(
            "match_integers",
            &tb(&match_integers(&lens, std == "2019")),
            "t",
            std,
            3000,
        );
    }
}

/// `?=` and `?/=` with every pair of element values (the `-` don't care, `U` against `X`, `L`
/// and `H` against `0` and `1`), one pair or two pairs in a vector, equal and unequal lengths with
/// every value as the sign element of a `signed` operand, and the constant of `x ?= "10--"`.
fn qeq_pairs() -> String {
    let all = "UX01ZWLH-";
    let mut body = String::new();
    let mut put = |f: &str, x: &str, y: &str, lx: usize, ly: usize| {
        for op in ["?=", "?/="] {
            let _ = writeln!(
                body,
                "    report img({f}(\"{x}\", {lx}, false) {op} {f}(\"{y}\", {ly}, true));"
            );
        }
    };
    for n in [1usize, 3, 65] {
        for a in all.chars() {
            for b in all.chars() {
                let mut ats = vec![0, n / 2, n - 1];
                ats.dedup();
                for at in ats {
                    let (mut x, mut y) = ("1".repeat(n).into_bytes(), "1".repeat(n).into_bytes());
                    x[at] = a as u8;
                    y[at] = b as u8;
                    let (x, y) = (String::from_utf8_lossy(&x), String::from_utf8_lossy(&y));
                    put("uv", &x, &y, n + 2, n + 3);
                    put("gv", &x, &y, n - 1, n + 4);
                }
            }
        }
    }
    // Two pairs: the fold is U before X before a mismatch wherever they stand.
    let six = "UX01-";
    for a in six.chars() {
        for b in six.chars() {
            for c in six.chars() {
                for d in six.chars() {
                    put("uv", &format!("{a}{c}"), &format!("{b}{d}"), 5, 5);
                }
            }
        }
    }
    // Unequal lengths: the shorter is extended with zeros or its sign element.
    for sgn in all.chars() {
        for (xn, yn) in [(3usize, 5usize), (5, 3), (64, 70), (3, 65)] {
            for y in ["0", "1", "-", "H", "L"] {
                let x = format!("{sgn}{}", "10".repeat(xn).get(..xn - 1).unwrap_or(""));
                let yv = y.repeat(yn);
                put("gv", &x, &yv, xn + 1, yn + 7);
                put("uv", &x, &yv, xn + 1, yn + 7);
                put("gv", &yv, &x, yn + 7, xn + 1);
            }
        }
    }
    // Don't care in the constant, in the variable and in both.
    for n in [8usize, 16, 64, 65, 128] {
        let alt: String = (0..n).map(|i| if i % 2 == 0 { '1' } else { '0' }).collect();
        let dc = format!("10{}", "-".repeat(n - 2));
        let var = format!("{}--", &alt[..n - 2]);
        for (x, y) in [
            (&alt, &dc),
            (&dc, &alt),
            (&var, &dc),
            (&dc, &dc),
            (&alt, &var),
        ] {
            put("uv", x, y, n - 1, n - 1);
            put("gv", x, y, n - 1, n - 1);
        }
    }
    // Over 1024 elements, past every word-wise path.
    let n = 1030;
    let mut extra = String::new();
    let alt: String = (0..n).map(|i| if i % 2 == 0 { '1' } else { '0' }).collect();
    let dc = format!("10{}", "-".repeat(n - 2));
    let meta = format!("{}X", &alt[..n - 1]);
    for (x, y) in [(&alt, &dc), (&meta, &alt), (&dc, &meta)] {
        put("uv", x, y, n, n);
        put("gv", x, y, n, n + 1);
        for (f, a, b) in [("uv", x, y), ("gv", y, x)] {
            let (a, b) = (
                format!("{f}(\"{a}\", {n}, false)"),
                format!("{f}(\"{b}\", {n}, true)"),
            );
            for op in ["?<", "?>="] {
                let _ = writeln!(extra, "    report img({a} {op} {b});");
            }
            for g in ["std_match", "maximum", "minimum"] {
                let _ = writeln!(extra, "    report img({g}({a}, {b}));");
            }
        }
    }
    body.push_str(&extra);
    // The integer forms against a vector with don't cares, metavalues and weak bits.
    for f in ["uv", "gv"] {
        for x in ["10--", "1X0-", "U010", "LH01", "----", "ZW01", "HLHL"] {
            for i in ["0", "5", "9", "15", "16", "255"] {
                for op in ["?=", "?/="] {
                    let _ = writeln!(body, "    report img({f}(\"{x}\", 3, false) {op} {i});");
                    let _ = writeln!(body, "    report img({i} {op} {f}(\"{x}\", 3, true));");
                }
            }
        }
    }
    body
}

/// The don't care and metavalue rules of `?=` and `?/=` against the bodies.
#[test]
fn native_qeq_dont_care_matches_bodies() {
    diff_run("qeq_pairs", &tb(&qeq_pairs()), "t", "2008", 3000);
}

/// A negative `NATURAL` operand is the body's subtype error; each case is its own run.
#[test]
fn native_match_negative_natural_matches_bodies() {
    for stmt in [
        "img(uv(\"101\", 2, true) ?= (-1))",
        "img((-1) ?< uv(\"101\", 2, true))",
        "img(maximum(uv(\"101\", 2, true), (-1)))",
        "img(minimum((-1), uv(\"\", -1, true)))",
        "img(uv(\"X1\", 1, true) ?>= (-2147483647 - 1))",
    ] {
        let body = format!("    report \"before\";\n    report {stmt};\n    report \"after\";\n");
        diff_run("match_natural", &tb(&body), "t", "2008", 1);
    }
}

/// Wrap generated statements into the shared header (entity `t`).
fn tb(body: &str) -> String {
    format!("{HEAD}{body}    wait;\n  end process;\nend architecture;\n")
}

/// `uniform` (seeds, the generator's sequence, both out-of-range reports) and the hexadecimal
/// and octal string conversions (padding with `Z`, every metavalue, both directions).
#[test]
fn native_uniform_and_strings_match_bodies() {
    let mut g = Gen {
        rng: Rng(5),
        out: String::new(),
        n: 0,
    };
    for lit in [
        "ZZZZZ", "Z1", "ZZZ", "LHLH", "UUUU", "1", "ZZZZ0", "0ZZZ", "-W1X",
    ] {
        for f in ["to_hstring", "to_ostring"] {
            g.case(&format!("sv(\"{lit}\", 9, false)"), f);
            g.case(&format!("sv(\"{lit}\", 9, true)"), f);
        }
    }
    for _ in 0..iters(120) {
        let n = 1 + g.rng.below(40) as usize;
        let (s, b) = g.vec_n(n);
        for f in ["to_hstring", "to_ostring"] {
            g.case(&format!("sv(\"{s}\", {b})"), f);
            g.case(&format!("uv(\"{s}\", {b})"), f);
            g.case(&format!("gv(\"{s}\", {b})"), f);
        }
    }
    let body = g.out;
    diff_run("strings", &tb(&body), "t", "2008", 500 / scale());
    // Null vector: the body's index error is the answer.
    let nul = tb("    report \"null \" & to_hstring(sv(\"\", 0, false));\n");
    diff_run("strings_null", &nul, "t", "2008", 0);

    let mut u = String::from(
        "library ieee;\nuse ieee.math_real.all;\nentity u is end entity;\narchitecture a of u is\nbegin\n  process\n    variable s1, s2 : positive := 1;\n    variable x : real;\n  begin\n",
    );
    for (a, b) in [
        (1, 1),
        (2147483562, 2147483398),
        (2147483563, 5),
        (5, 2147483399),
        (123456, 654321),
        (53668, 52774),
    ] {
        let _ = writeln!(
            u,
            "    s1 := {a}; s2 := {b};\n    for i in 1 to 300 loop\n      uniform(s1, s2, x);\n      report real'image(x) & \" \" & integer'image(s1) & \" \" & integer'image(s2);\n    end loop;"
        );
    }
    u.push_str("    wait;\n  end process;\nend architecture;\n");
    diff_run("uniform", &u, "u", "2008", 1500 / scale());
}

const BIT_HEAD: &str = r#"library ieee;
use ieee.numeric_bit.all;
entity t is end entity;
architecture a of t is
  function img (x : bit_vector) return string is
    variable r : string(1 to x'length);
    variable k : integer := 0;
  begin
    for i in x'range loop
      k := k + 1;
      r(k) := bit'image(x(i))(2);
    end loop;
    return integer'image(x'left) & "," & integer'image(x'right) & "," &
           boolean'image(x'ascending) & ":" & r;
  end function;
  function img (x : unsigned) return string is
  begin return img(bit_vector(x)); end function;
  function img (x : signed) return string is
  begin return img(bit_vector(x)); end function;
  function img (b : boolean) return string is
  begin return boolean'image(b); end function;
  function img (i : integer) return string is
  begin return integer'image(i); end function;
  function bv (s : string; l : integer; asc : boolean) return bit_vector is
    variable d : bit_vector(l downto l - s'length + 1);
    variable u : bit_vector(l to l + s'length - 1);
  begin
    if asc then
      for i in u'range loop u(i) := bit'value("'" & s(s'left + i - l) & "'"); end loop;
      return u;
    else
      for i in d'range loop d(i) := bit'value("'" & s(s'left + l - i) & "'"); end loop;
      return d;
    end if;
  end function;
  function uv (s : string; l : integer; asc : boolean) return unsigned is
  begin return unsigned(bv(s, l, asc)); end function;
  function gv (s : string; l : integer; asc : boolean) return signed is
  begin return signed(bv(s, l, asc)); end function;
begin
  process
  begin
"#;

/// `numeric_bit` against its bodies: the arithmetic, relational, conversion and shift natives
/// on bit arrays, with the package's own warning texts.
#[test]
fn native_numeric_bit_matches_bodies() {
    let mut g = Gen {
        rng: Rng(21),
        out: String::new(),
        n: 0,
    };
    for _ in 0..iters(40) {
        for (ty, t) in [("uv", "unsigned"), ("gv", "signed")] {
            let mk = |g: &mut Gen, n: Option<usize>| {
                let len = n.unwrap_or_else(|| g.len());
                let s: String = (0..len)
                    .map(|_| b"01"[g.rng.below(2) as usize] as char)
                    .collect();
                let l = g.rng.below(9) as i64 + 97;
                let asc = g.rng.below(2) == 0;
                format!("{ty}(\"{s}\", {l}, {asc})")
            };
            let a = mk(&mut g, None);
            let n = g.len();
            let b = if g.rng.below(3) == 0 {
                a.clone()
            } else {
                mk(&mut g, Some(n))
            };
            let b = if g.rng.below(2) == 0 {
                mk(&mut g, None)
            } else {
                b
            };
            for op in ["+", "-", "*"] {
                g.case(&format!("{a} {op} {b}"), "img");
            }
            for op in ["<", "<=", ">", ">=", "=", "/="] {
                g.case(&format!("{a} {op} {b}"), "img");
            }
            let nat = t == "unsigned";
            let d = g.nonzero_bit(ty);
            for op in ["/", "rem", "mod"] {
                g.case(&format!("{a} {op} {d}"), "img");
            }
            for op in ["+", "-", "*"] {
                let i = g.int(nat);
                g.case(&format!("{a} {op} {i}"), "img");
                let i = g.int(nat);
                g.case(&format!("{i} {op} {a}"), "img");
            }
            for op in ["<", "<=", ">", ">=", "=", "/="] {
                let i = g.int(nat);
                g.case(&format!("{a} {op} {i}"), "img");
                let i = g.int(nat);
                g.case(&format!("{i} {op} {a}"), "img");
            }
            let s = g.size();
            g.case(&format!("resize({a}, {s})"), "img");
            let i = g.int(nat);
            let s = g.size();
            let f = if nat { "to_unsigned" } else { "to_signed" };
            g.case(&format!("{f}({i}, {s})"), "img");
            let c = g.rng.below(20);
            for f in ["shift_left", "shift_right", "rotate_left", "rotate_right"] {
                if a.contains("\"\"") && f.starts_with("rotate") {
                    continue;
                }
                g.case(&format!("{f}({a}, {c})"), "img");
            }
            g.case(&format!("{a} sll {c}"), "img");
            g.case(&format!("{a} sra {c}"), "img");
            g.case(&format!("minimum({a}, {b})"), "img");
            g.case(&format!("maximum({b}, {a})"), "img");
            if !nat {
                g.case(&format!("-{a}"), "img");
                g.case(&format!("abs {a}"), "img");
            }
            let s: String = (0..g.rng.below(31))
                .map(|_| b"01"[g.rng.below(2) as usize] as char)
                .collect();
            g.case(&format!("to_integer({ty}(\"{s}\", 40, false))"), "img");
        }
    }
    for (ty, v) in [
        ("gv", "1"),
        ("gv", "0"),
        ("uv", "1"),
        ("gv", ""),
        ("uv", ""),
    ] {
        g.case(&format!("to_integer({ty}(\"{v}\", 0, false))"), "img");
        g.case(
            &format!("{ty}(\"{v}\", 0, false) < {ty}(\"01\", 1, true)"),
            "img",
        );
        g.case(
            &format!("{ty}(\"{v}\", 0, false) /= {ty}(\"01\", 1, true)"),
            "img",
        );
        g.case(&format!("{ty}(\"{v}\", 0, false) + 1"), "img");
        g.case(
            &format!("minimum({ty}(\"{v}\", 0, false), {ty}(\"01\", 1, true))"),
            "img",
        );
        if ty == "gv" {
            g.case(&format!("-{ty}(\"{v}\", 0, false)"), "img");
            g.case(&format!("abs {ty}(\"{v}\", 0, false)"), "img");
        }
    }
    let src = format!(
        "{BIT_HEAD}{}    wait;\n  end process;\nend architecture;\n",
        g.out
    );
    diff_run("numeric_bit", &src, "t", "2008", 1000 / scale());
}

/// 64-bit INTEGER (2019): conversions of wide vectors, large integer operands, `floor` and
/// `ceil` near INTEGER'HIGH and the NATURAL error text, native against body (#416).
#[test]
fn native_integer64_matches_bodies_2019() {
    let mut b = String::new();
    let big = [
        "0",
        "1",
        "2147483647",
        "2147483648",
        "4294967295",
        "4294967296",
        "2863311530",
        "1099511627776",
        "4611686018427387903",
        "4611686018427387904",
        "9223372036854775807",
    ];
    for v in big {
        for f in ["to_unsigned", "to_signed"] {
            let (ty, n, var) = if f == "to_signed" {
                ("signed", 64, "g")
            } else {
                ("unsigned", 64, "u")
            };
            let _ = writeln!(
                b,
                "    {var} := {ty}'({f}({v}, {n})); report \"{f} {v} \" & integer'image(to_integer({var}));"
            );
            let _ = writeln!(
                b,
                "    report \"{f} {v} small \" & to_hstring(std_ulogic_vector({f}({v}, 40)));"
            );
        }
        let _ = writeln!(
            b,
            "    report \"cmp {v} \" & boolean'image(unsigned'(to_unsigned({v}, 64)) > {v}) & boolean'image({v} < signed'(to_signed({v}, 64)));"
        );
        let _ = writeln!(
            b,
            "    report \"add {v} \" & integer'image(to_integer(unsigned'(to_unsigned({v}, 63)) + 1));"
        );
        let _ = writeln!(
            b,
            "    report \"conv {v} \" & to_hstring(std_ulogic_vector(conv_unsigned({v}, 64))) & to_hstring(std_ulogic_vector(conv_signed(-{v}, 64))) & to_hstring(conv_std_logic_vector({v}, 70));"
        );
    }
    for v in [
        "-9223372036854775807",
        "(-9223372036854775807 - 1)",
        "-4294967296",
        "-2147483649",
    ] {
        let _ = writeln!(
            b,
            "    report \"neg {v} \" & to_hstring(std_ulogic_vector(to_signed({v}, 64))) & integer'image(to_integer(to_signed({v}, 64)));"
        );
        let _ = writeln!(
            b,
            "    report \"convneg {v} \" & to_hstring(std_ulogic_vector(conv_signed({v}, 64))) & to_hstring(std_ulogic_vector(conv_unsigned({v}, 64)));"
        );
    }
    for x in [
        "2147483646.5",
        "2147483647.5",
        "3.0e9",
        "-3.0e9",
        "4.0e18",
        "9.0e18",
        "1.0e19",
        "-9.0e18",
        "123.5",
        "-0.5",
    ] {
        for f in ["floor", "ceil", "round", "trunc"] {
            let _ = writeln!(b, "    report \"{f} {x} \" & real'image({f}({x}));");
        }
    }
    let _ = writeln!(
        b,
        "    report \"nat \" & integer'image(to_integer(unsigned'(shift_left(unsigned'(to_unsigned(1, 70)), 69)(62 downto 0))));"
    );
    let _ = writeln!(b, "    n := -1; u := shift_left(u, n);");
    let src = format!(
        "library ieee;\nuse ieee.std_logic_1164.all;\nuse ieee.numeric_std.all;\nuse ieee.std_logic_arith.conv_unsigned;\nuse ieee.std_logic_arith.conv_signed;\nuse ieee.std_logic_arith.conv_std_logic_vector;\nuse ieee.math_real.all;\nentity t is end entity;\narchitecture a of t is\nbegin\n  process\n    variable u : unsigned(63 downto 0);\n    variable g : signed(63 downto 0);\n    variable n : integer;\n  begin\n{b}    wait;\n  end process;\nend architecture;\n"
    );
    diff_run("int64", &src, "t", "2019", 100);
}

/// Vectors longer than one plane word: the word-wise paths against the bodies. Lengths at and
/// around every word boundary, mostly clean `0 1` vectors with some `L`/`H` and some metavalues,
/// all-zero, all-one and one-hot patterns, a small operand against a wide one, integer operands
/// of both signs, resizes and shifts across words.
#[test]
fn native_wide_matches_bodies() {
    let mut g = Gen {
        rng: Rng(77),
        out: String::new(),
        n: 0,
    };
    const LENS: [usize; 14] = [1, 8, 63, 64, 65, 66, 100, 127, 128, 129, 192, 193, 300, 513];
    fn wide(g: &mut Gen, ty: &str, len: usize) -> String {
        let kind = g.rng.below(12);
        let s: String = (0..len)
            .map(|i| match kind {
                0 => '0',
                1 => '1',
                2 => {
                    if i == g.rng.below(len as u64) as usize {
                        '1'
                    } else {
                        '0'
                    }
                }
                3 => b"01LH"[g.rng.below(4) as usize] as char,
                4 if g.rng.below(40) == 0 => b"UXZW-"[g.rng.below(5) as usize] as char,
                _ => b"01"[g.rng.below(2) as usize] as char,
            })
            .collect();
        let (l, asc) = (g.rng.below(9) as i64 + 600, g.rng.below(2) == 0);
        format!("{ty}(\"{s}\", {l}, {asc})")
    }
    fn small_int(g: &mut Gen, nat: bool) -> String {
        let v: i64 = match g.rng.below(5) {
            0 => 0,
            1 => 2147483647,
            2 => g.rng.below(5) as i64,
            _ => g.rng.below(70000) as i64,
        };
        if !nat && g.rng.below(2) == 0 {
            format!("(-{v})")
        } else {
            v.to_string()
        }
    }
    for _ in 0..iters(30) {
        let n = LENS[g.rng.below(LENS.len() as u64) as usize];
        let m = if g.rng.below(3) == 0 {
            LENS[g.rng.below(LENS.len() as u64) as usize]
        } else {
            n
        };
        for op in ["and", "nand", "or", "nor", "xor", "xnor"] {
            let (x, y) = (wide(&mut g, "sv", n), wide(&mut g, "sv", n));
            g.case(&format!("{x} {op} {y}"), "img");
            let (x, y) = (wide(&mut g, "uv", n), wide(&mut g, "uv", n));
            g.case(&format!("{x} {op} {y}"), "img");
        }
        let x = wide(&mut g, "sv", n);
        g.case(&format!("not {x}"), "img");
        let x = wide(&mut g, "gv", n);
        g.case(&format!("not {x}"), "img");
        for (ty, nat) in [("uv", true), ("gv", false)] {
            let (a, b) = (wide(&mut g, ty, n), wide(&mut g, ty, m));
            for op in ["+", "-"] {
                g.case(&format!("{a} {op} {b}"), "img");
            }
            for op in ["<", "<=", ">", ">=", "=", "/="] {
                g.case(&format!("{a} {op} {b}"), "img");
                g.case(&format!("{a} {op} {a}"), "img");
            }
            for op in ["+", "-", "*"] {
                let i = small_int(&mut g, nat);
                g.case(&format!("{a} {op} {i}"), "img");
                g.case(&format!("{i} {op} {a}"), "img");
            }
            for op in ["<", "<=", ">", ">=", "=", "/="] {
                let i = small_int(&mut g, nat);
                g.case(&format!("{a} {op} {i}"), "img");
                g.case(&format!("{i} {op} {a}"), "img");
            }
            let s = LENS[g.rng.below(LENS.len() as u64) as usize];
            g.case(&format!("resize({a}, {s})"), "img");
            g.case(&format!("resize({a}, {})", s + 1), "img");
            let i = small_int(&mut g, nat);
            let f = if nat { "to_unsigned" } else { "to_signed" };
            g.case(&format!("{f}({i}, {s})"), "img");
            for f in ["shift_left", "shift_right"] {
                for c in [
                    0,
                    1,
                    7,
                    63,
                    64,
                    65,
                    130,
                    n as u64 - 1,
                    n as u64,
                    n as u64 + 3,
                ] {
                    g.case(&format!("{f}({a}, {c})"), "img");
                }
            }
            g.case(&format!("rotate_left({a}, 70)"), "img");
            g.case(&format!("{a} sll 5"), "img");
            g.case(&format!("{a} srl 66"), "img");
        }
        // to_integer of a wide vector whose value fits an INTEGER.
        let low: String = (0..31)
            .map(|_| b"01"[g.rng.below(2) as usize] as char)
            .collect();
        let zeros = "0".repeat(n.max(32) - 31);
        let ones = "1".repeat(n.max(32) - 31);
        g.case(
            &format!("to_integer(uv(\"{zeros}{low}\", 600, false))"),
            "img",
        );
        g.case(
            &format!("to_integer(gv(\"{zeros}{low}\", 600, false))"),
            "img",
        );
        g.case(
            &format!("to_integer(gv(\"{ones}{low}\", 600, false))"),
            "img",
        );
    }
    diff_run("wide", &tb(&g.out), "t", "2008", 1000 / scale());
}

/// Dividends and divisors of `n` elements: zero, all ones, the most negative value, one, an
/// alternating pattern, weak bits and weak signs, metavalues. A divisor that is zero (no `1` or
/// `H`, no metavalue) is the body's error and ends the run, so it is not in the second list.
fn div_pats(n: usize, bit: bool) -> (Vec<String>, Vec<String>) {
    let alt: String = (0..n).map(|i| if i % 2 == 0 { '1' } else { '0' }).collect();
    let mut v = vec![
        "0".repeat(n),
        "1".repeat(n),
        format!("1{}", "0".repeat(n - 1)),
        format!("{}1", "0".repeat(n - 1)),
        alt.clone(),
    ];
    if !bit {
        v.extend([
            alt.replace('1', "H").replace('0', "L"),
            format!("H{}", "0".repeat(n - 1)),
            format!("L{}", "1".repeat(n - 1)),
            format!("X{}", &alt[1..]),
        ]);
    }
    v.sort();
    v.dedup();
    let divisors = v
        .iter()
        .filter(|s| s.contains(['1', 'H', 'U', 'X']))
        .cloned()
        .collect();
    (v, divisors)
}

/// `/`, `rem` and `mod` of two vectors of one type around the limits of the one-word path (1, 63,
/// 64, 65 elements), narrower, as wide and wider than each other, extremes, weak signs, metavalues,
/// null, both directions with a lower bound other than zero.
fn div_limits_body(f: &str, pairs: &[(usize, usize)], lower: &[(usize, usize)]) -> String {
    let mut body = String::new();
    for &(a, b) in pairs {
        let (xs, _) = div_pats(a, false);
        let (_, ys) = div_pats(b, false);
        for x in &xs {
            for y in &ys {
                for op in ["/", "rem", "mod"] {
                    let _ = writeln!(
                        body,
                        "    report img({f}(\"{x}\", {}, true) {op} {f}(\"{y}\", {}, false));",
                        a - 1,
                        b - 1
                    );
                }
            }
        }
    }
    // Non-zero lower bounds, both directions for each operand.
    for &(a, b) in lower {
        let (xs, _) = div_pats(a, false);
        let (_, ys) = div_pats(b, false);
        for x in xs.iter().step_by(2) {
            for y in ys.iter().step_by(2) {
                for (da, db) in [("false", "true"), ("true", "false")] {
                    for op in ["/", "rem", "mod"] {
                        let _ = writeln!(
                            body,
                            "    report img({f}(\"{x}\", {}, {da}) {op} {f}(\"{y}\", {}, {db}));",
                            a + 6,
                            b + 2
                        );
                    }
                }
            }
        }
    }
    for op in ["/", "rem", "mod"] {
        for (x, y) in [
            ("\"\", -1", "\"101\", 2"),
            ("\"101\", 2", "\"\", -1"),
            ("\"\", -1", "\"\", -1"),
        ] {
            let _ = writeln!(body, "    report img({f}({x}, true) {op} {f}({y}, false));");
        }
    }
    let wide: String = (0..72)
        .map(|i| if i % 3 == 0 { '1' } else { '0' })
        .collect();
    for (hi, lo, hj, lj) in [
        (40, 9, 70, 39),
        (70, 40, 5, 4),
        (66, 33, 63, 34),
        (63, 0, 0, 0),
    ] {
        for op in ["/", "rem", "mod"] {
            let _ = writeln!(
                body,
                "    report img({f}(\"{wide}\", 71, false)({hi} downto {lo}) {op} {f}(\"{wide}\", 71, false)({hj} downto {lj}));"
            );
        }
    }
    body
}

const DIV_PAIRS: [(usize, usize); 16] = [
    (1, 1),
    (1, 63),
    (63, 1),
    (3, 5),
    (5, 3),
    (32, 33),
    (64, 64),
    (1, 64),
    (64, 1),
    (65, 1),
    (1, 65),
    (65, 64),
    (64, 65),
    (65, 65),
    (70, 3),
    (3, 70),
];
const DIV_LOWER: [(usize, usize); 6] = [(3, 5), (33, 31), (1, 64), (64, 1), (65, 3), (3, 65)];

#[test]
fn native_div_limits_unsigned_match_bodies() {
    let body = div_limits_body("uv", &DIV_PAIRS, &DIV_LOWER);
    diff_run("div_limits_u", &tb(&body), "t", "2008", 1500);
}

#[test]
fn native_div_limits_signed_match_bodies() {
    let body = div_limits_body("gv", &DIV_PAIRS, &DIV_LOWER);
    diff_run("div_limits_s", &tb(&body), "t", "2008", 1500);
}

/// The division bodies of 1993 and 2019 are those of 2008 (the integers are 32 and 64 bits): a
/// smaller run of the same shapes in each.
#[test]
fn native_div_limits_other_revisions_match_bodies() {
    let pairs = [(1, 1), (3, 5), (5, 3), (64, 65), (65, 64), (64, 1)];
    for f in ["uv", "gv"] {
        let body = div_limits_body(f, &pairs, &[(33, 31)]);
        for std in ["1993", "2019"] {
            diff_run("div_limits_rev", &tb(&body), "t", std, 400);
        }
    }
}

/// A zero divisor is the body's error report and the run ends there; each case is its own run.
#[test]
fn native_div_by_zero_matches_bodies() {
    for n in [1, 4, 64, 65, 130] {
        let zero = "0".repeat(n);
        let weak = "L".repeat(n);
        for f in ["uv", "gv"] {
            for op in ["/", "rem", "mod"] {
                for z in [&zero, &weak] {
                    let stmt = format!(
                        "img({f}(\"{}\", {}, true) {op} {f}(\"{z}\", {}, false))",
                        "1".repeat(n),
                        n - 1,
                        n - 1
                    );
                    let body = format!(
                        "    report \"before\";\n    report {stmt};\n    report \"after\";\n"
                    );
                    diff_run("div_zero", &tb(&body), "t", "2008", 1);
                }
            }
        }
    }
    // An integer divisor or dividend of zero, the vector forms of the same.
    for stmt in [
        "img(uv(\"0101\", 3, true) / 0)",
        "img(uv(\"0101\", 3, true) rem 0)",
        "img(uv(\"0101\", 3, true) mod 0)",
        "img(gv(\"1101\", 3, true) / 0)",
        "img(gv(\"1101\", 3, true) rem 0)",
        "img(gv(\"1101\", 3, true) mod 0)",
        "img(5 / uv(\"0000\", 3, true))",
        "img(5 rem uv(\"0000\", 3, true))",
        "img(5 mod uv(\"0000\", 3, true))",
        "img(-5 / gv(\"0000\", 3, true))",
        "img(-5 rem gv(\"0000\", 3, true))",
        "img(-5 mod gv(\"0000\", 3, true))",
        "img(uv(\"01\", 1, true) / 0)",
        "img(gv(\"1\", 0, true) mod 0)",
        "img(uv(\"01\", 1, true) / (2 ** 31 - 1))",
        "img(uv(\"01\", 1, true) rem uv(\"00000000000000000000000000000000000000000000000000000000000000000\", 64, true))",
    ] {
        let body = format!("    report \"before\";\n    report {stmt};\n    report \"after\";\n");
        for std in ["2008", "1993"] {
            diff_run("div_zero_int", &tb(&body), "t", std, 1);
        }
    }
}

/// `/`, `rem` and `mod` of a vector and an integer, both orders: the integer fits, just fits or
/// just does not (the body's warning or quotient of zero), negative and extreme values, weak bits,
/// metavalues, null, both directions with a lower bound. Zero divisors are separate runs.
fn div_integer_body(types: &[(&str, bool)], wide_int: bool, lens: &[usize], bit: bool) -> String {
    let mut body = String::new();
    for &(f, signed) in types {
        for &n in lens {
            let (xs, ys) = div_pats(n, bit);
            let top: i128 = if wide_int { 1 << 63 } else { 1 << 31 };
            let mut ints: Vec<i128> = vec![0, 1, 3, 255, top - 1];
            if signed {
                ints.extend([-1, -2, -7, -top]);
            }
            for k in [n as u32, (n as u32).saturating_sub(1)] {
                if (1..63).contains(&k) {
                    let w: i128 = if signed { 1i128 << k >> 1 } else { 1i128 << k };
                    ints.extend([w - 1, w, w + 1]);
                    if signed {
                        ints.extend([-w, -w - 1]);
                    }
                }
            }
            ints.retain(|v| *v < top && *v >= if signed { -top } else { 0 });
            ints.sort();
            ints.dedup();
            let lit = |v: i128| {
                if v == -(1i128 << 63) {
                    "(-9223372036854775807 - 1)".to_string()
                } else if v == -(1i128 << 31) && !wide_int {
                    "(-2147483647 - 1)".to_string()
                } else if v < 0 {
                    format!("(-{})", -v)
                } else {
                    v.to_string()
                }
            };
            // The second shape (ascending, a lower bound) with a third of the patterns.
            for (l, asc, step) in [(n - 1, false, 1), (n + 6, true, 3)] {
                for x in xs.iter().step_by(step) {
                    let vec = format!("{f}(\"{x}\", {l}, {asc})");
                    for v in ints.iter().filter(|v| **v != 0) {
                        for op in ["/", "rem", "mod"] {
                            let _ = writeln!(body, "    report img({vec} {op} {});", lit(*v));
                        }
                    }
                }
                for y in ys.iter().step_by(step) {
                    let vec = format!("{f}(\"{y}\", {l}, {asc})");
                    for v in &ints {
                        for op in ["/", "mod"] {
                            let _ = writeln!(body, "    report img({} {op} {vec});", lit(*v));
                        }
                        // The body of `natural rem unsigned` has no RESIZE: it is a length error
                        // for an integer wider than the vector (its own run).
                        if signed || *v < (1i128 << n) {
                            let _ = writeln!(body, "    report img({} rem {vec});", lit(*v));
                        }
                    }
                }
            }
        }
        let _ = writeln!(body, "    report img({f}(\"\", -1, true) / 5);");
        let _ = writeln!(body, "    report img({f}(\"\", -1, true) rem 5);");
        let _ = writeln!(body, "    report img({f}(\"\", -1, true) mod 5);");
        let _ = writeln!(body, "    report img(5 / {f}(\"\", -1, false));");
        let _ = writeln!(body, "    report img(5 mod {f}(\"\", -1, false));");
        if signed {
            let _ = writeln!(body, "    report img({f}(\"\", -1, true) / (-5));");
            let _ = writeln!(body, "    report img((-5) mod {f}(\"\", -1, false));");
        }
    }
    body
}

const U: [(&str, bool); 1] = [("uv", false)];
const G: [(&str, bool); 1] = [("gv", true)];

#[test]
fn native_div_integer_forms_unsigned_match_bodies() {
    for (std, lens) in [
        ("2008", vec![1, 2, 31, 33, 64, 65]),
        ("2019", vec![1, 33, 64]),
        ("1993", vec![2, 33]),
    ] {
        let body = div_integer_body(&U, std == "2019", &lens, false);
        diff_run("div_integer_u", &tb(&body), "t", std, 300);
    }
}

#[test]
fn native_div_integer_forms_signed_match_bodies() {
    for (std, lens) in [
        ("2008", vec![1, 2, 31, 33, 64, 65]),
        ("2019", vec![1, 33, 64]),
        ("1993", vec![2, 33]),
    ] {
        let body = div_integer_body(&G, std == "2019", &lens, false);
        diff_run("div_integer_s", &tb(&body), "t", std, 300);
    }
}

#[test]
fn native_div_integer_numeric_bit_matches_bodies() {
    let body = div_integer_body(&[("uv", false), ("gv", true)], false, &[1, 2, 33], true);
    let src = format!("{BIT_HEAD}{body}    wait;\n  end process;\nend architecture;\n");
    diff_run("div_integer_bit", &src, "t", "2008", 300);
}

/// `natural rem unsigned` and its signed twin with an integer wider than the vector: the unsigned
/// body is a length error (no RESIZE), the signed one resizes. Native and body must agree.
#[test]
fn native_div_rem_integer_wider_than_vector_matches_bodies() {
    for stmt in [
        "img(300 rem uv(\"101\", 2, true))",
        "img(8 rem uv(\"101\", 2, false))",
        "img(300 rem uv(\"1\", 0, true))",
        "img(300 rem gv(\"101\", 2, true))",
        "img(-300 rem gv(\"101\", 2, false))",
        "img(300 mod uv(\"101\", 2, true))",
        "img(300 / uv(\"101\", 2, true))",
        "img(uv(\"101\", 2, true) rem 300)",
        "img(5 rem uv(\"\", -1, true))",
        "img(5 rem uv(\"\", -1, false))",
        "img(5 rem uv(\"01\", 1, true))",
    ] {
        let body = format!("    report \"before\";\n    report {stmt};\n    report \"after\";\n");
        for std in ["2008", "2019", "1993"] {
            diff_run("div_rem_wide_int", &tb(&body), "t", std, 1);
        }
    }
}

/// The word-wise path against the bodies: lengths at and around each word boundary up to 1024,
/// one operand small and the other wide, the dividend smaller than, equal to and larger than the
/// divisor, powers of two and all-ones operands, random operands of both signs.
#[test]
fn native_div_wide_matches_bodies() {
    let mut rng = Rng(1109);
    const LENS: [usize; 17] = [
        1, 2, 33, 63, 64, 65, 66, 100, 127, 128, 129, 192, 193, 300, 513, 1023, 1024,
    ];
    let shape = |len: usize, k: u64, rng: &mut Rng| -> String {
        (0..len)
            .map(|i| match k {
                0 => '1',
                1 => {
                    if i == len - 1 {
                        '1'
                    } else {
                        '0'
                    }
                }
                2 => {
                    if i == 0 {
                        '1'
                    } else {
                        '0'
                    }
                }
                3 => {
                    if i % 64 == 0 {
                        '1'
                    } else {
                        '0'
                    }
                }
                // Mostly zero above a few low bits: a small divisor, or a sparse dividend.
                4 => {
                    if i < 9 {
                        b"01"[rng.below(2) as usize] as char
                    } else {
                        '0'
                    }
                }
                _ => b"01"[rng.below(2) as usize] as char,
            })
            .collect()
    };
    let mut body = String::new();
    for _ in 0..iters(150) {
        let a = LENS[rng.below(LENS.len() as u64) as usize];
        let b = if rng.below(3) == 0 {
            LENS[rng.below(LENS.len() as u64) as usize]
        } else {
            a
        };
        for f in ["uv", "gv"] {
            let (ka, kb) = (rng.below(7), rng.below(7));
            let x = shape(a, ka, &mut rng);
            let mut y = shape(b, kb, &mut rng);
            if !y.contains('1') {
                y.replace_range(0..1, "1");
            }
            let l = rng.below(9) as i64 + 1100;
            for op in ["/", "rem", "mod"] {
                let _ = writeln!(
                    body,
                    "    report img({f}(\"{x}\", {l}, false) {op} {f}(\"{y}\", {l}, true));"
                );
            }
        }
    }
    diff_run("div_wide", &tb(&body), "t", "2008", 400 / scale());
}

/// A negative `NATURAL` operand of `/`, `rem` or `mod` is the body's subtype error, also when the
/// vector is null or has a metavalue; each case is its own run since the error ends it.
#[test]
fn native_div_negative_natural_matches_bodies() {
    for op in ["/", "rem", "mod"] {
        for (x, n) in [
            ("uv(\"101\", 2, true)", "(-1)"),
            ("uv(\"\", -1, true)", "(-3)"),
            ("uv(\"X1\", 1, true)", "(-2147483647 - 1)"),
        ] {
            for stmt in [format!("img({x} {op} {n})"), format!("img({n} {op} {x})")] {
                let body =
                    format!("    report \"before\";\n    report {stmt};\n    report \"after\";\n");
                diff_run("div_natural", &tb(&body), "t", "2008", 1);
            }
        }
    }
}

/// Small vectors against integers around their range, both orders: wider than the vector (the
/// bodies divide at the larger length and resize back: the `mod` truncation warnings, a quotient
/// of zero, the sign element of the larger length), just fitting, and fitting.
fn div_small_body(f: &str, signed: bool, bit: bool) -> String {
    let mut body = String::new();
    let mut vecs: Vec<String> = Vec::new();
    for n in 2..=3usize {
        for v in 0..1u32 << n {
            vecs.push(format!("{v:0n$b}"));
        }
    }
    vecs.extend(["0000", "1000", "1111", "0111", "0001", "1010"].map(String::from));
    if !bit {
        vecs.extend(["1H", "L1", "H0L", "X10", "1L0H"].map(String::from));
    }
    let ints: &[i64] = &[
        -33, -17, -9, -8, -5, -4, -3, -2, -1, 0, 1, 2, 3, 4, 5, 7, 8, 9, 15, 16, 17, 31, 32, 100,
    ];
    for x in &vecs {
        let n = x.len();
        let vec = format!("{f}(\"{x}\", {}, false)", n - 1);
        for &v in ints.iter().filter(|v| signed || **v >= 0) {
            let lit = if v < 0 {
                format!("(-{})", -v)
            } else {
                v.to_string()
            };
            if v != 0 {
                for op in ["/", "rem", "mod"] {
                    let _ = writeln!(body, "    report img({vec} {op} {lit});");
                }
            }
            // A divisor of zero (weak ones too) ends the run; `natural rem unsigned` is a length error.
            if x.contains(['1', 'H', 'X']) {
                for op in ["/", "mod"] {
                    let _ = writeln!(body, "    report img({lit} {op} {vec});");
                }
                if signed || (v as u64) < 1 << n {
                    let _ = writeln!(body, "    report img({lit} rem {vec});");
                }
            }
        }
    }
    body
}

#[test]
fn native_div_small_unsigned_match_bodies() {
    diff_run(
        "div_small_u",
        &tb(&div_small_body("uv", false, false)),
        "t",
        "2008",
        1500,
    );
}

#[test]
fn native_div_small_signed_match_bodies() {
    diff_run(
        "div_small_s",
        &tb(&div_small_body("gv", true, false)),
        "t",
        "2008",
        1500,
    );
}

#[test]
fn native_div_small_numeric_bit_matches_bodies() {
    let body = format!(
        "{}{}",
        div_small_body("uv", false, true),
        div_small_body("gv", true, true)
    );
    let src = format!("{BIT_HEAD}{body}    wait;\n  end process;\nend architecture;\n");
    diff_run("div_small_bit", &src, "t", "2008", 1500);
}

/// The predefined `?=` and `?/=` of `std_ulogic` and `std_ulogic_vector` (LRM 9.2.3, the AND of
/// the element matches): every pair of element values alone and as two pairs (a mismatch beats
/// `U`, which beats `X`), lengths 63, 64, 65, 128 and 1030 with the values in each word, weak
/// bits, both directions, non-zero bounds, null operands; `pairs` false leaves out the two pairs.
fn predefined_match(pairs: bool) -> String {
    let all = "UX01ZWLH-";
    let mut body = String::new();
    let mut put = |x: &str, y: &str, lx: i64, ly: i64| {
        for op in ["?=", "?/="] {
            let _ = writeln!(
                body,
                "    report img(sv(\"{x}\", {lx}, false) {op} sv(\"{y}\", {ly}, true));"
            );
        }
    };
    for a in all.chars() {
        for b in all.chars() {
            put(&a.to_string(), &b.to_string(), 0, 3);
            for c in "UX01-".chars().filter(|_| pairs) {
                for d in "UX01-".chars() {
                    put(&format!("{a}{c}"), &format!("{b}{d}"), 7, 2);
                }
            }
        }
    }
    for n in [63usize, 64, 65, 128, 1030] {
        let alt: String = (0..n).map(|i| if i % 2 == 0 { '1' } else { 'L' }).collect();
        put(&alt, &alt, n as i64 - 1, n as i64);
        put(&alt, &alt.replace('1', "H"), n as i64 + 5, n as i64 + 2);
        for at in [0, n / 2, n - 1] {
            for (a, b) in [('X', '1'), ('U', '1'), ('0', '1'), ('-', '0'), ('W', 'U')] {
                let mut x = alt.clone().into_bytes();
                x[at] = a as u8;
                let mut y = alt.clone().into_bytes();
                y[n - 1 - at] = b as u8;
                let (x, y) = (String::from_utf8_lossy(&x), String::from_utf8_lossy(&y));
                put(&x, &y, n as i64 - 1, n as i64 + 1);
            }
        }
    }
    put("", "", 0, 5);
    // The scalar forms through a function, so nothing folds.
    for a in all.chars() {
        for b in all.chars() {
            for op in ["?=", "?/="] {
                let _ = writeln!(body, "    report img(cv('{a}') {op} cv('{b}'));");
            }
        }
    }
    body
}

#[test]
fn native_predefined_match_matches_bodies() {
    diff_run("pmatch", &tb(&predefined_match(true)), "t", "2008", 4400);
    diff_run("pmatch", &tb(&predefined_match(false)), "t", "2019", 450);
    // Different lengths, one of them null: the error of the operator.
    for (x, y) in [("10", "101"), ("", "1"), ("1", "")] {
        for op in ["?=", "?/="] {
            let body = format!(
                "    report \"before\";\n    report img(sv(\"{x}\", 3, false) {op} sv(\"{y}\", 2, true));\n    report \"after\";\n"
            );
            assert_eq!(diff_run("pmatch_len", &tb(&body), "t", "2008", 1), Some(1));
        }
    }
}
