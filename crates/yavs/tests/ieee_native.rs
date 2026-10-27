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
            for op in ["+", "-"] {
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
        }
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

/// Analyze `src` (entity `ent`) and run it with natives on and off: same stdout, stderr and
/// exit status, and at least `min` lines of output.
fn diff_run(tag: &str, src: &str, ent: &str, std: &str, min: usize) {
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
            for op in ["+", "-"] {
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
            for op in ["+", "-"] {
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
