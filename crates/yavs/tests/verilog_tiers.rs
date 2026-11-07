//! Verilog processes on the compiled tiers against the interpreter (#776): random modules of
//! expressions of every operator over registers of widths 1 to 100 with x and z bits, shown with
//! `%b` at every clock (a process's first activation is interpreted, the next ones run compiled
//! code), and a clocked process of non-blocking assignments, `if` and `case` over them. The
//! output of `--tier=clift`, of `--tier=auto` compiling every function at its first run, and of
//! `--tier=llvm` where it is built must be the interpreter's, byte for byte, also where a value
//! just falls off the two-state path (one x bit).
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
    let names: Vec<&str> = regs.iter().map(|g| g.name.as_str()).collect();
    format!(
        "`timescale 1ns/1ps
module vt;
  reg clk = 0;
  integer i = {i};
{decl}  always #5 clk = ~clk;
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
    let mut tiers: Vec<&[&str]> = Vec::new();
    #[cfg(feature = "clift")]
    tiers.extend([
        &["--tier=clift"][..],
        &["--tier=auto", "--tier-heat=1,0,0"][..],
    ]);
    #[cfg(feature = "llvm")]
    tiers.push(&["--tier=llvm"][..]);
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
    }
}
