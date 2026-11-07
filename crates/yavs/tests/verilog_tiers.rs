//! Verilog processes on the compiled tiers against the interpreter (#776): random modules of
//! expressions of every operator over registers of widths 1 to 100 with x and z bits, shown with
//! `%b` at every clock (a process's first activation is interpreted, the next ones run compiled
//! code), and a clocked process of non-blocking assignments, `if` and `case` over them. The
//! `@*`, `@(a or b)` and `posedge`/`negedge` of registers whose bit 0 goes through x and z. The
//! output of `--tier=clift`, of `--tier=auto` compiling every function at its first run, and of
//! `--tier=llvm` where it is built must be the interpreter's, byte for byte, also where a value
//! just falls off the two-state path (one x bit). Random three-state buses (several drivers with
//! strengths, `tri0`/`tri1` pulls, overlapping part drivers) the same way, with the resolved
//! `std_ulogic` VHDL sees on output ports.
#![cfg(any(feature = "clift", feature = "llvm"))]

mod common;
use common::tempdir;

use std::process::Command;

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
    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[self.below(xs.len() as u64) as usize]
    }
}

struct Reg {
    name: String,
    w: u32,
}

/// A sized literal of `w` bits: mostly known (the compiled two-state paths), sometimes with one
/// x or z bit (just off them), sometimes with any.
fn literal(r: &mut Rng, w: u32) -> String {
    let mode = r.below(10);
    let one = r.below(w as u64) as u32;
    let bits: String = (0..w)
        .rev()
        .map(|i| match (mode, r.below(10)) {
            (7 | 8, _) if i == one => ['x', 'z'][r.below(2) as usize],
            (9, 0) => 'x',
            (9, 1) => 'z',
            _ => ['0', '1'][r.below(2) as usize],
        })
        .collect();
    let s = if r.below(3) == 0 { "s" } else { "" };
    format!("{w}'{s}b{bits}")
}

fn operand(r: &mut Rng, regs: &[Reg]) -> String {
    match r.below(5) {
        0 => {
            let w = [1, 3, 8, 16, 33][r.below(5) as usize];
            literal(r, w)
        }
        1 => {
            let g = &regs[r.below(regs.len() as u64) as usize];
            match r.below(3) {
                0 => format!("{}[i]", g.name),
                1 if g.w >= 2 => {
                    let hi = 1 + r.below(g.w as u64 - 1) as u32;
                    let lo = r.below(hi as u64) as u32;
                    format!("{}[{hi}:{lo}]", g.name)
                }
                _ => g.name.clone(),
            }
        }
        _ => regs[r.below(regs.len() as u64) as usize].name.clone(),
    }
}

fn expr(r: &mut Rng, regs: &[Reg], d: u32) -> String {
    if d == 0 {
        return operand(r, regs);
    }
    let (a, b) = (expr(r, regs, d - 1), expr(r, regs, d - 1));
    match r.below(12) {
        0..=4 => {
            let op = r.pick(&[
                "+", "-", "*", "&", "|", "^", "~^", "<<", ">>", ">>>", "<<<", "<", "<=", ">", ">=",
                "==", "!=", "===", "!==", "&&", "||", "/", "%", "**",
            ]);
            format!("({a} {op} {b})")
        }
        5 | 6 => {
            let op = r.pick(&["~", "-", "!", "&", "|", "^", "~&", "~|", "~^"]);
            format!("({op}{a})")
        }
        7 => {
            let c = expr(r, regs, d - 1);
            format!("({c} ? {a} : {b})")
        }
        8 => format!("{{{a}, {b}}}"),
        9 => format!(
            "($signed({a}) {} $signed({b}))",
            r.pick(&["<", ">=", ">>>", "+", "*", "<="])
        ),
        10 => format!("$unsigned({a})"),
        _ => operand(r, regs),
    }
}

fn module(seed: u64) -> String {
    let mut r = Rng(seed);
    let mut regs = Vec::new();
    let mut decl = String::new();
    for k in 0..8 {
        let w = [1, 2, 7, 8, 16, 31, 32, 63, 64, 65, 100][r.below(11) as usize];
        let signed = if r.below(2) == 0 { "signed " } else { "" };
        let init = literal(&mut r, w);
        decl += &format!("  reg {signed}[{}:0] r{k} = {init};\n", w - 1);
        regs.push(Reg {
            name: format!("r{k}"),
            w,
        });
    }
    let mut shows = String::new();
    for _ in 0..32 {
        shows += &format!("    $display(\"%b\", {});\n", expr(&mut r, &regs, 3));
    }
    let mut clocked = String::new();
    for g in &regs {
        let e = expr(&mut r, &regs, 2);
        clocked += &match r.below(3) {
            0 => format!("    if ({}) {} <= {e};\n", expr(&mut r, &regs, 1), g.name),
            1 => format!(
                "    {} ({}) {} : {} <= {e}; default: {} <= ~{};\n    endcase\n",
                r.pick(&["case", "casez", "casex"]),
                expr(&mut r, &regs, 1),
                literal(&mut r, 4),
                g.name,
                g.name,
                g.name
            ),
            _ => format!("    {} <= {e};\n", g.name),
        };
    }
    // `@*`, `@(a or b)` and edges of bits that go through x and z (Table 9-9).
    let star = expr(&mut r, &regs, 2);
    let mut names: Vec<&str> = regs.iter().map(|g| g.name.as_str()).collect();
    names.extend(["w1", "c0", "c1"]);
    format!(
        "`timescale 1ns/1ps
module vt;
  reg clk = 0;
  integer i = {i};
{decl}  reg [15:0] w1;
  reg [7:0] c0 = 0, c1 = 0;
  always #5 clk = ~clk;
  always @* w1 = {star};
  always @(r0 or r1) c0 = c0 + 1;
  always @(posedge r2 or negedge r3) c1 <= c1 + 1;
  always @(posedge clk) begin
{clocked}    i <= i + 1;
  end
  always @(negedge clk) begin
    $display(\"{fmt}\", {list});
{shows}  end
  initial #100 $finish;
endmodule
",
        i = r.below(10),
        star = star,
        fmt = vec!["%b"; names.len()].join(" "),
        list = names.join(", "),
    )
}

const TB: &str = "entity tb is end entity;
architecture sim of tb is
  component vt is end component;
begin
  u : vt;
end architecture;
";

/// A module of resolved nets of one width: per net two to four continuous drivers that give `z`
/// when their enable is not 1, each with a drive strength, on a `wire`, a `tri0`, a `tri1` (both
/// output ports) and a `wire` driven by two overlapping parts; enables and values (with x and
/// z) change at every clock. Shown with `%b` (Verilog reads `TO_X01Z`), counted per event, and
/// reported by VHDL on the ports (`L`, `H`, `W` stay).
fn bus_module(seed: u64) -> (String, String) {
    let mut r = Rng(seed);
    let w = [1, 3, 8, 33, 64, 65, 100][r.below(7) as usize];
    let st = [
        "",
        "",
        "(strong0, strong1) ",
        "(weak0, weak1) ",
        "(pull0, pull1) ",
        "(supply0, supply1) ",
        "(strong0, weak1) ",
        "(weak0, strong1) ",
        "(highz0, strong1) ",
        "(strong0, highz1) ",
    ];
    let (mut decl, mut drv, mut step) = (String::new(), String::new(), Vec::new());
    let mut k = 0;
    for net in ["b0", "b1", "b2"] {
        for _ in 0..2 + r.below(3) {
            decl += &format!("  reg e{k} = 0;\n  reg [{}:0] a{k};\n", w - 1);
            drv += &format!(
                "  assign {}{net} = e{k} ? a{k} : {{{w}{{1'bz}}}};\n",
                r.pick(&st)
            );
            k += 1;
        }
    }
    // Two parts of b3 that share bit `m`.
    let m = r.below(w as u64) as u32;
    decl += &format!("  reg [{}:0] a{k}, a{};\n", w - 1, k + 1);
    drv += &format!("  assign b3[{}:{m}] = a{k}[{}:{m}];\n", w - 1, w - 1);
    drv += &format!("  assign b3[{m}:0] = a{}[{m}:0];\n", k + 1);
    for _ in 0..16 {
        let mut s = String::new();
        for j in 0..k {
            let e = [
                "1'b0", "1'b0", "1'b0", "1'b1", "1'b1", "1'b1", "1'bx", "1'bz",
            ][r.below(8) as usize];
            s += &format!("e{j} <= {e}; a{j} <= {}; ", literal(&mut r, w));
        }
        for j in k..k + 2 {
            s += &format!("a{j} <= {}; ", literal(&mut r, w));
        }
        step.push(s);
    }
    let cases: String = step
        .iter()
        .enumerate()
        .map(|(i, s)| format!("      {i}: begin {s}end\n"))
        .collect();
    let v = format!(
        "`timescale 1ns/1ps
module vb (output [{h}:0] b0, output tri0 [{h}:0] b1, output tri1 [{h}:0] b2);
  reg clk = 0;
  integer i = 0, n0 = 0, n1 = 0, n2 = 0, n3 = 0;
  wire [{h}:0] b3;
{decl}{drv}  always #5 clk = ~clk;
  always @(posedge clk) begin
    case (i)
{cases}    endcase
    i <= i + 1;
  end
  always @(b0) n0 = n0 + 1;
  always @(posedge b1) n1 = n1 + 1;
  always @(negedge b2) n2 = n2 + 1;
  always @(b3 or b1) n3 = n3 + 1;
  always @(negedge clk) $display(\"%b %b %b %b %0d %0d %0d %0d\", b0, b1, b2, b3, n0, n1, n2, n3);
  initial #180 $finish;
endmodule
",
        h = w - 1
    );
    let tb = format!(
        "library ieee;
use ieee.std_logic_1164.all;
entity tb is end entity;
architecture sim of tb is
  component vb is port (b0, b1, b2 : out std_logic_vector({h} downto 0)); end component;
  signal b0, b1, b2 : std_logic_vector({h} downto 0);
begin
  u : vb port map (b0, b1, b2);
  process
  begin
    wait on b0, b1, b2;
    report to_string(b0) & \" \" & to_string(b1) & \" \" & to_string(b2);
  end process;
end architecture;
",
        h = w - 1
    );
    (v, tb)
}

fn tiers() -> Vec<&'static [&'static str]> {
    let mut tiers: Vec<&[&str]> = Vec::new();
    #[cfg(feature = "clift")]
    tiers.extend([
        &["--tier=clift"][..],
        &["--tier=auto", "--tier-heat=1,0,0"][..],
    ]);
    #[cfg(feature = "llvm")]
    tiers.push(&["--tier=llvm"][..]);
    tiers
}

#[test]
fn resolved_nets_match_the_interpreter() {
    let t = tempdir("v-tiers-bus");
    let yavs = |args: &[&str]| {
        let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
            .current_dir(&*t)
            .args(args)
            .output()
            .unwrap();
        (
            o.status.code(),
            String::from_utf8_lossy(&o.stdout).into_owned(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
        )
    };
    let mut weak = false;
    for seed in 1..=12u64 {
        let (src, tb) = bus_module(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15));
        std::fs::write(t.join("vb.v"), &src).unwrap();
        std::fs::write(t.join("tb.vhd"), &tb).unwrap();
        let (code, _, err) = yavs(&["analyze", "--work=work:lib", "vb.v", "tb.vhd"]);
        assert_eq!(code, Some(0), "seed {seed}: {err}\n{src}");
        let want = yavs(&["run", "--work=work:lib", "--tier=interp", "tb"]);
        assert_eq!(want.0, Some(0), "seed {seed}: {}\n{src}", want.2);
        assert!(want.1.lines().count() > 30, "seed {seed}: {}", want.1);
        weak |= want.1.contains('H') || want.1.contains('L');
        for tier in tiers() {
            let got =
                yavs(&[&["run", "--work=work:lib", "--no-cache"][..], tier, &["tb"]].concat());
            assert_eq!(got.1, want.1, "seed {seed} {tier:?}:\n{src}\n{}", got.2);
            assert_eq!(got.0, want.0, "seed {seed} {tier:?}: {}", got.2);
        }
    }
    // The pulls and weak drivers reach VHDL as `L` and `H` in some seed.
    assert!(weak);
}

#[test]
fn compiled_tiers_match_the_interpreter() {
    let t = tempdir("v-tiers");
    let yavs = |args: &[&str]| {
        let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
            .current_dir(&*t)
            .args(args)
            .output()
            .unwrap();
        (
            o.status.code(),
            String::from_utf8_lossy(&o.stdout).into_owned(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
        )
    };
    let tiers = tiers();
    for seed in 1..=24u64 {
        let src = module(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15));
        std::fs::write(t.join("vt.v"), &src).unwrap();
        std::fs::write(t.join("tb.vhd"), TB).unwrap();
        let (code, _, err) = yavs(&["analyze", "--work=work:lib", "vt.v", "tb.vhd"]);
        assert_eq!(code, Some(0), "seed {seed}: {err}\n{src}");
        let want = yavs(&["run", "--work=work:lib", "--tier=interp", "tb"]);
        assert_eq!(want.0, Some(0), "seed {seed}: {}\n{src}", want.2);
        assert!(want.1.lines().count() > 200, "seed {seed}: {}", want.1);
        for tier in &tiers {
            let got =
                yavs(&[&["run", "--work=work:lib", "--no-cache"][..], tier, &["tb"]].concat());
            assert_eq!(got.1, want.1, "seed {seed} {tier:?}:\n{src}\n{}", got.2);
            assert_eq!(got.0, want.0, "seed {seed} {tier:?}: {}", got.2);
        }
        // With the work library's compilation cache: cold, then warm (code linked from it).
        if seed <= 3 {
            for tier in &tiers {
                for run in ["cold", "warm"] {
                    let got = yavs(&[&["run", "--work=work:lib"][..], tier, &["tb"]].concat());
                    assert_eq!(got.1, want.1, "seed {seed} {tier:?} {run}:\n{}", got.2);
                }
            }
        }
    }
}

/// The run-time LRM checks (`tests/verilog/lrm/*_eval_*.v` with `// output:` lines) under a VHDL
/// testbench on every tier of the build: the output their headers give.
#[test]
fn run_time_lrm_checks_on_every_tier() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/verilog/lrm");
    let t = tempdir("v-lrm-tiers");
    let mut tiers: Vec<&[&str]> = vec![&["--tier=interp"][..]];
    #[cfg(feature = "clift")]
    tiers.extend([
        &["--tier=clift"][..],
        &["--tier=auto", "--tier-heat=1,0,0"][..],
    ]);
    #[cfg(feature = "llvm")]
    tiers.push(&["--tier=llvm"][..]);
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "v"))
        .collect();
    files.sort();
    let mut n = 0;
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap();
        let head: Vec<&str> = text.lines().take_while(|l| l.starts_with("//")).collect();
        let want: Vec<&str> = head
            .iter()
            .filter_map(|l| l.strip_prefix("// output:"))
            .map(|l| l.strip_prefix(' ').unwrap_or(l))
            .collect();
        if want.is_empty() || head.iter().any(|l| l.starts_with("// elab:")) {
            continue;
        }
        n += 1;
        let name = text
            .lines()
            .filter_map(|l| l.trim_start().strip_prefix("module "))
            .next_back()
            .and_then(|l| l.split(|c: char| !c.is_alphanumeric() && c != '_').next())
            .unwrap();
        std::fs::copy(&f, t.join("m.v")).unwrap();
        let tb = format!(
            "entity tb is end entity;\narchitecture sim of tb is\n  component {name} is end component;\nbegin\n  u : {name};\nend architecture;\n"
        );
        std::fs::write(t.join("tb.vhd"), tb).unwrap();
        let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
            .current_dir(&*t)
            .args(["analyze", "--work=work:lib", "m.v", "tb.vhd"])
            .output()
            .unwrap();
        assert!(
            o.status.success(),
            "{}: {}",
            f.display(),
            String::from_utf8_lossy(&o.stderr)
        );
        for tier in &tiers {
            let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
                .current_dir(&*t)
                .args(
                    [
                        &["run", "--work=work:lib", "--no-cache", "--stop-time=1ms"][..],
                        tier,
                        &["tb"],
                    ]
                    .concat(),
                )
                .output()
                .unwrap();
            let got = String::from_utf8_lossy(&o.stdout);
            let got: Vec<&str> = got.lines().collect();
            assert_eq!(
                got,
                want,
                "{} {tier:?}: {}",
                f.display(),
                String::from_utf8_lossy(&o.stderr)
            );
        }
    }
    assert!(n >= 10, "only {n} run checks");
}
