//! Native `ieee` operators against their VHDL bodies (`yavs_ir::ieee`).
//!
//! A generated testbench applies every native operator to random operands, metavalues, null
//! arrays, odd lengths and both directions, and reports each result with its bounds. It runs
//! twice, natives on and off (`YAVS_NO_NATIVE_IEEE`); stdout, stderr and the exit code must be
//! identical. `YAVS_IEEE_KEEP=dir` keeps the generated sources.

use std::fmt::Write;
use std::path::Path;
use std::process::{Command, Output};

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
        let l = self.rng.below(9) as i64 - 3;
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
        }
        let x = g.operand("sv");
        g.case(&format!("not {x}"), "img");
        for f in ["to_x01", "to_x01z", "to_ux01"] {
            g.case(&format!("{f}({x})"), "img");
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
    if !native {
        c.env("YAVS_NO_NATIVE_IEEE", "1");
    }
    c.output().expect("run yavs")
}

fn compare(seed: u64, std: &str) {
    let dir = std::env::temp_dir().join(format!("yavs-ieee-{}-{seed}-{std}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let src = testbench(seed, 25);
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
        ns.lines().count() > 1000,
        "too few results:\n{}",
        String::from_utf8_lossy(&n.stderr)
    );
    for (i, (x, y)) in ns.lines().zip(vs.lines()).enumerate() {
        assert_eq!(x, y, "line {i} (seed {seed}, std {std})");
    }
    assert_eq!(ns.lines().count(), vs.lines().count());
    assert_eq!(n.stderr, v.stderr);
    assert_eq!(n.status.code(), v.status.code());
    let _ = std::fs::remove_dir_all(&dir);
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
    let dir = std::env::temp_dir().join(format!("yavs-ieee-err-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("e.vhd"), ERRORS).expect("write");
    let a = run(
        &dir,
        &["analyze", "--std=2008", "--work=work:lib", "e.vhd"],
        true,
    );
    assert!(a.status.success());
    for k in 0..9 {
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
    let _ = std::fs::remove_dir_all(&dir);
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
    let dir = std::env::temp_dir().join(format!("yavs-ieee-edge-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
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
    let _ = std::fs::remove_dir_all(&dir);
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
    let dir = std::env::temp_dir().join(format!("yavs-ieee-math-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
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
    let _ = std::fs::remove_dir_all(&dir);
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
    for _ in 0..150 {
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
    let dir = std::env::temp_dir().join(format!("yavs-ieee-arith-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
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
        ns.lines().count() > 800,
        "{}",
        String::from_utf8_lossy(&n.stderr)
    );
    for (i, (x, y)) in ns.lines().zip(vs.lines()).enumerate() {
        assert_eq!(x, y, "line {i}");
    }
    assert_eq!(ns.lines().count(), vs.lines().count());
    assert_eq!(n.stderr, v.stderr);
    assert_eq!(n.status.code(), v.status.code());
    let _ = std::fs::remove_dir_all(&dir);
}
