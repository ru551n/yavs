//! Front-end tests: sources are written to a scratch directory and analysed through the public API.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use yavs_verilog::{
    AnalysisResult, CODES, Dir, Lang, Options, Severity, Span, Timescale, UnitKind, analyze,
};

static N: AtomicUsize = AtomicUsize::new(0);

fn scratch() -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "fe-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, text).unwrap();
    p
}

fn run(src: &str) -> AnalysisResult {
    run_with(src, &Options::default())
}

fn run_with(src: &str, opts: &Options) -> AnalysisResult {
    let d = scratch();
    let f = write(&d, "t.v", src);
    analyze(&[f], opts)
}

fn errors(r: &AnalysisResult) -> Vec<(&'static str, usize)> {
    r.diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .map(|d| (d.code, d.span.map_or(0, |s| line(r, s))))
        .collect()
}

/// One-based line of a span start in its original file.
fn line(r: &AnalysisResult, s: Span) -> usize {
    let t = std::fs::read(&r.files[s.file]).unwrap();
    t[..s.start as usize]
        .iter()
        .filter(|&&c| c == b'\n')
        .count()
        + 1
}

#[track_caller]
fn ok(src: &str) -> AnalysisResult {
    let r = run(src);
    let e: Vec<_> = r
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .map(|d| format!("{} {}", d.code, d.message))
        .collect();
    assert!(e.is_empty(), "unexpected errors: {e:?}");
    for d in &r.diagnostics {
        assert!(
            CODES.iter().any(|c| c.code == d.code),
            "code {} is not in CODES",
            d.code
        );
    }
    r
}

/// The source must give exactly this error code at this line.
#[track_caller]
fn bad(src: &str, code: &str, line_no: usize) -> AnalysisResult {
    let r = run(src);
    let e = errors(&r);
    assert_eq!(
        e.len(),
        1,
        "expected one error, got {:?}",
        r.diagnostics
            .iter()
            .map(|d| (d.code, &d.message))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        (e[0].0, e[0].1),
        (code, line_no),
        "{:?}",
        r.diagnostics[0].message
    );
    assert!(CODES.iter().any(|c| c.code == code));
    r
}

#[test]
fn codes_are_unique_and_documented() {
    let mut seen = std::collections::HashSet::new();
    for c in CODES {
        assert!(seen.insert(c.code), "{} twice", c.code);
        assert!(c.code.starts_with('E') || c.code.starts_with('W'));
        assert!(
            c.code[1..]
                .parse::<u32>()
                .is_ok_and(|n| (4000..5000).contains(&n))
        );
        assert!(!c.title.is_empty() && c.explain.len() > 40, "{}", c.code);
        assert!(!c.explain.contains('\u{2014}') && !c.title.contains('\u{2014}'));
    }
}

/// Every code written in the sources is registered.
#[test]
fn every_code_in_the_sources_is_in_the_table() {
    let mut n = 0;
    for src in [
        include_str!("../src/pre.rs"),
        include_str!("../src/lex.rs"),
        include_str!("../src/parse.rs"),
        include_str!("../src/stmt.rs"),
        include_str!("../src/analysis.rs"),
        include_str!("../src/lib.rs"),
    ] {
        for (i, _) in src.match_indices("\"E4").chain(src.match_indices("\"W4")) {
            let code = &src[i + 1..i + 6];
            assert!(
                CODES.iter().any(|c| c.code == code),
                "{code} is not in CODES"
            );
            n += 1;
        }
    }
    assert!(n > 40);
    // and every registered code is used somewhere
    let all = [
        include_str!("../src/pre.rs"),
        include_str!("../src/lex.rs"),
        include_str!("../src/parse.rs"),
        include_str!("../src/stmt.rs"),
        include_str!("../src/analysis.rs"),
        include_str!("../src/lib.rs"),
    ]
    .concat();
    // E4001 is raised at elaboration, not here.
    for c in CODES.iter().filter(|c| c.code != "E4001") {
        assert!(
            all.contains(&format!("\"{}\"", c.code)),
            "{} is never emitted",
            c.code
        );
    }
}

#[test]
fn ansi_module_with_everything_in_one_place() {
    let r = ok(r#"
`timescale 1 ps / 1 ps
`celldefine
module Top #(parameter [7:0] INIT = 8'h00, parameter integer N = 4, parameter S = "TRUE", localparam W = N * 2)
(
  output reg [W-1:0] q = 0,
  output wire o,
  inout io,
  input wire signed [3:0] a, b,
  input clk, rst
);
  localparam MODULE_NAME = "Top";
  tri0 glblGSR = glbl.GSR;
  wire [N:0] w1, w2;
  reg [7:0] mem [0:15];
  integer i;
  time t;
  real r;
  realtime rt;
  event ev;
  genvar g;
  supply0 gnd;
  wire (strong1, weak0) #(1, 2) strong_net = 1'b1;
  assign #1 o = &a ? 1'b1 : 1'bz;
  assign (weak0, weak1) w1 = {N{1'b0}}, w2 = {a, b};
  and #(1) g1 (o, a[0], a[1]), g2 (w1[0], a[2], a[3]);
  bufif1 (o, w1[1], clk);
  pullup (o);
  pulldown pd (io);
  nmos (io, a[0], b[0]);
  Sub #(.P(1), .Q("x")) u0 (.x(a), .y(), .z(q[0]));
  Sub #(1, 2) u1 [0:1] (a, , q[0]);
  defparam u0.P = 3;
  always @(posedge clk or negedge rst) begin : blk
    reg tmp;
    if (!rst) q <= #1 0;
    else begin
      q <= q + 1;
      tmp = a[0] ^ b[0];
    end
  end
  always @* i = i + 1;
  always @(*) begin
    case (a)
      4'b00?? , 4'bxxxx: i = 1;
      default: i = 2;
    endcase
    casex (a) 4'b1x0z: i = 3; endcase
    casez (a) 4'b1?0z: i = 4; endcase
  end
  always @(a or b, clk) if (a == b) i = 0; else if (a != b) i = 1; else i = 2;
  initial begin
    #5 q = 0;
    @(posedge clk) q = 1;
    q = #3 2;
    q = @(posedge clk) 3;
    q = repeat (2) @(posedge clk) 4;
    repeat (3) @(negedge clk);
    forever #10 begin end
  end
  initial fork : f
    #1 i = 0;
    #2 i = 1;
  join
  initial begin
    for (i = 0; i < 4; i = i + 1) mem[i] = i;
    while (i > 0) i = i - 1;
    wait (rst) i = 5;
    -> ev;
    disable blk;
    force q = 1;
    release q;
    assign q = 2;
    deassign q;
    $display("v=%b %h %0d %t", q, q, i, $time);
    $finish;
    $readmemh("x.hex", mem);
    t = $time;
    r = $itor(i) * 1.5e-3;
  end
  function automatic [7:0] f1 (input [7:0] x, input y);
    reg [7:0] z;
    begin z = x ** 2; f1 = z >>> 1; end
  endfunction
  function integer f2; input x; f2 = x; endfunction
  function real f3; input real x; f3 = x; endfunction
  task automatic t1 (input a, output b); begin b = a; end endtask
  task t2; input a; output b; reg c; begin #1 b = a; end endtask
  generate
    genvar k;
    for (k = 0; k < 2; k = k + 1) begin : gen
      wire x = a[k];
    end
    if (N > 1) begin : g_if
      wire y;
    end else wire z;
    case (N)
      1: wire c1;
      2, 3: begin : c23 wire c2; end
      default: wire c3;
    endcase
  endgenerate
  if (N == 4) wire bare_if;
  (* keep = "true", mark *) wire attributed;
  x_udp (o, a[0], a[1]);
  specify
    specparam PATHPULSE$ = 0, tD = 1:2:3;
    specparam PATHPULSE$CLK$Q = (0, 0);
    pulsestyle_onevent o;
    showcancelled o;
    noshowcancelled o;
    (clk => q[0]) = (1, 2);
    (clk *> q) = 100;
    (a +=> o) = 1;
    (a -*> o) = (1:2:3, 4:5:6);
    (posedge clk => (q[0] +: a[0])) = (1, 1);
    (negedge clk => (q[1] : b[0])) = 1;
    if (rst) (a, b *> o) = 2;
    ifnone (a => o) = 3;
    $setuphold (posedge clk, a, 0:0:0, 0:0:0, , , , dclk, da);
    $setup (a &&& rst, posedge clk, 1);
    $hold (edge [01, 10] clk, a, 1);
    $width (negedge clk, 5, 0, ev);
    $period (posedge clk, 10);
    $recovery (posedge rst, posedge clk, 1);
  endspecify
endmodule
`endcelldefine

primitive x_udp (o, a, b);
  output o; input a, b;
  table
    0 0 : 0;
    1 1 : 1;
    ? 0 : 0;
  endtable
endprimitive
primitive seq (output reg q, input d, clk);
  initial q = 1'b0;
  table
    // d clk : q : q+
       0 (01) : ? : 0;
       1 (01) : ? : 1;
       ? (1x) : ? : -;
       * ?    : ? : -;
       ? (?0) : ? : -;
  endtable
endprimitive
"#);
    assert_eq!(r.units.len(), 3);
    let t = &r.units[0];
    assert_eq!((t.name.as_str(), t.kind), ("Top", UnitKind::Module));
    assert!(t.celldefine);
    assert_eq!(
        t.timescale,
        Timescale {
            unit_fs: 1000,
            precision_fs: 1000
        }
    );
    let names: Vec<_> = t.ports.iter().map(|p| (p.name.as_str(), p.dir)).collect();
    assert_eq!(
        names,
        [
            ("q", Dir::Output),
            ("o", Dir::Output),
            ("io", Dir::Inout),
            ("a", Dir::Input),
            ("b", Dir::Input),
            ("clk", Dir::Input),
            ("rst", Dir::Input)
        ]
    );
    assert_eq!(t.ports[0].range, Some(("(W - 1)".into(), "0".into())));
    assert_eq!(r.units[1].kind, UnitKind::Primitive);
    assert_eq!(
        r.units[1].ports.iter().map(|p| p.dir).collect::<Vec<_>>(),
        [Dir::Output, Dir::Input, Dir::Input]
    );
    assert_eq!(r.units[2].ports.len(), 3);
}

#[test]
fn non_ansi_ports_take_their_direction_and_range_from_the_body() {
    let r = ok(
        "module m (a, b, .c(d), e); input a; output [7:0] b; inout d; input signed e; wire x; endmodule",
    );
    let p: Vec<_> = r.units[0]
        .ports
        .iter()
        .map(|p| (p.name.as_str(), p.dir, p.range.clone()))
        .collect();
    assert_eq!(p[0], ("a", Dir::Input, None));
    assert_eq!(p[1], ("b", Dir::Output, Some(("7".into(), "0".into()))));
    assert_eq!(p[2], ("c", Dir::Inout, None));
    assert_eq!(p[3].1, Dir::Input);
    bad("module m (a, b);\n input a;\nendmodule", "E4202", 1);
    bad(
        "module m (a);\n input a;\n output z;\nendmodule",
        "E4202",
        3,
    );
    bad("module m (a);\n input a;\n input a;\nendmodule", "E4202", 3);
}

#[test]
fn macros_conditionals_and_define_options() {
    let r = ok(r#"
`define W 8
`define ADD(a, b) ((a) + (b))
`define TWO(x, y) `ADD(x, y) + `ADD(y,x)
`define LONG(a) a + \
   1
`define EMPTY
`ifdef NOPE
  this is not even Verilog
`ifndef INNER
  neither is this
`endif
`elsif FROM_CMD
module `W_NAME_UNUSED_IN_INACTIVE;
`else
`endif
`ifndef UNDEFINED_ONE
module m1 #(parameter P = `TWO(1, 2 `EMPTY) + `LONG(3)) (output [`W-1:0] o); endmodule
`else
module m_wrong; endmodule
`endif
`undef W
`ifdef W
module m_wrong2; endmodule
`endif
module m2; endmodule
"#);
    let n: Vec<_> = r.units.iter().map(|u| u.name.as_str()).collect();
    assert_eq!(n, ["m1", "m2"]);
    assert_eq!(
        r.units[0].ports[0].range,
        Some(("(8 - 1)".into(), "0".into()))
    );
    // -D picks the `elsif branch and is part of the context digest.
    let mut o = Options::default();
    o.defines.push(("FROM_CMD".into(), None));
    let d = scratch();
    let f = write(
        &d,
        "t.v",
        "`ifdef FROM_CMD\nmodule yes; endmodule\n`else\nmodule no; endmodule\n`endif\n",
    );
    let a = analyze(std::slice::from_ref(&f), &o);
    let b = analyze(&[f], &Options::default());
    assert_eq!(a.units[0].name, "yes");
    assert_eq!(b.units[0].name, "no");
    assert_ne!(a.units[0].context_digest, b.units[0].context_digest);
}

#[test]
fn macro_value_from_the_command_line() {
    let mut o = Options::default();
    o.defines.push(("WIDTH".into(), Some("12".into())));
    let r = run_with("module m (output [`WIDTH-1:0] o); endmodule", &o);
    assert_eq!(r.units[0].ports[0].range.as_ref().unwrap().0, "(12 - 1)");
}

#[test]
fn preprocessor_errors_have_codes_and_original_lines() {
    bad("module m;\n`ifdef A\nendmodule\n", "E4004", 2);
    bad("module m;\n`endif\nendmodule\n", "E4004", 2);
    bad("module m;\n`else\nendmodule\n", "E4004", 2);
    bad("module m;\nwire x = `NOPE;\nendmodule\n", "E4007", 2);
    bad(
        "`define F(a, b) a\nmodule m;\nwire x = `F(1);\nendmodule\n",
        "E4008",
        3,
    );
    bad(
        "`define F(a) a\nmodule m;\nwire x = `F;\nendmodule\n",
        "E4008",
        3,
    );
    bad("`define\nmodule m; endmodule\n", "E4008", 1);
    bad("`timescale 3ns/1ps\nmodule m; endmodule\n", "E4009", 1);
    bad("`timescale 1ps/1ns\nmodule m; endmodule\n", "E4009", 1);
    bad("`default_nettype foo\nmodule m; endmodule\n", "E4009", 1);
    bad("`uselib lib = x\nmodule m; endmodule\n", "E4010", 1);
    bad(
        "`define R `R\nmodule m;\nwire x = `R;\nendmodule\n",
        "E4011",
        3,
    );
    bad("module m;\nwire x = \"abc;\nendmodule\n", "E4014", 2);
    bad("module m; /* never closed\nendmodule\n", "E4014", 1);
    bad("module m;\nwire x = 4'b1 \u{a7};\nendmodule\n", "E4014", 2);
}

#[test]
fn errors_inside_a_macro_point_at_the_call() {
    let r = bad(
        "`define BAD )\nmodule m;\nwire x = `BAD;\nendmodule\n",
        "E4100",
        3,
    );
    assert!(r.diagnostics[0].message.contains("expected"));
}

#[test]
fn include_is_found_next_to_the_file_and_in_include_dirs() {
    let d = scratch();
    write(&d, "inc/defs.vh", "`define W 4\n");
    write(&d, "near.vh", "wire near;\n");
    let top = write(
        &d,
        "src/top.v",
        "`include \"defs.vh\"\nmodule m (output [`W-1:0] o);\n`include \"../near.vh\"\nendmodule\n",
    );
    let mut o = Options::default();
    o.include_dirs.push(d.join("inc"));
    let r = analyze(std::slice::from_ref(&top), &o);
    assert!(errors(&r).is_empty(), "{:?}", r.diagnostics);
    assert_eq!(r.units[0].included_files.len(), 2);
    assert!(r.files.iter().any(|f| f.ends_with("defs.vh")));
    // without -I: E4005 on line 1 of the including file, with the directories searched
    let r = analyze(std::slice::from_ref(&top), &Options::default());
    let e = &r.diagnostics[0];
    assert_eq!(e.code, "E4005");
    assert_eq!(line(&r, e.span.unwrap()), 1);
    assert!(e.notes[0].contains("searched"));
}

#[test]
fn an_error_in_an_include_points_into_the_included_file() {
    let d = scratch();
    write(&d, "bad.vh", "\n\nwire x = ;\n");
    let top = write(&d, "t.v", "module m;\n`include \"bad.vh\"\nendmodule\n");
    let r = analyze(&[top], &Options::default());
    let e = &r.diagnostics[0];
    assert_eq!(e.code, "E4100");
    let s = e.span.unwrap();
    assert!(r.files[s.file].ends_with("bad.vh"));
    assert_eq!(line(&r, s), 3);
}

#[test]
fn timescale_and_nettype_carry_over_between_files() {
    let d = scratch();
    let a = write(
        &d,
        "a.v",
        "`timescale 10ns / 100ps\n`default_nettype none\n`define SHARED 1\nmodule a; endmodule\n",
    );
    let b = write(
        &d,
        "b.v",
        "module b; wire w; `ifdef SHARED wire s; `endif endmodule\n",
    );
    let r = analyze(&[a, b], &Options::default());
    assert!(r.diagnostics.is_empty(), "{:?}", r.diagnostics);
    assert_eq!(
        r.units[0].timescale,
        Timescale {
            unit_fs: 10_000_000,
            precision_fs: 100_000
        }
    );
    assert_eq!(r.units[1].timescale, r.units[0].timescale);
    assert_eq!(format!("{:?}", r.units[1].default_nettype), "None");
    assert_ne!(r.units[0].context_digest, r.units[1].context_digest);
}

#[test]
fn missing_timescale_warns_once_and_uses_the_default() {
    let r = run("module a; endmodule\nmodule b; endmodule\n");
    let w: Vec<_> = r.diagnostics.iter().filter(|d| d.code == "W4006").collect();
    assert_eq!(w.len(), 1);
    assert_eq!(r.units[0].timescale, Timescale::DEFAULT);
    let o = Options {
        default_timescale: Timescale {
            unit_fs: 1_000,
            precision_fs: 1_000,
        },
        ..Options::default()
    };
    assert_eq!(
        run_with("module a; endmodule", &o).units[0]
            .timescale
            .unit_fs,
        1_000
    );
}

#[test]
fn encrypted_modules_are_recorded_with_one_warning_per_file() {
    let enc = "`timescale 1ps/1ps\nmodule SIP_X (\n input A,\n output [3:0] B\n);\n`pragma protect begin_protected\n`pragma protect key_keyowner = \"A\", key_method = \"rsa\"\n`pragma protect key_keyowner = \"B\"\nAAAA\nBBBB\n`pragma protect end_protected\n";
    let r = run(enc);
    assert!(errors(&r).is_empty());
    let w: Vec<_> = r.diagnostics.iter().filter(|d| d.code == "W4007").collect();
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].severity, Severity::Warning);
    assert_eq!(line(&r, w[0].span.unwrap()), 6);
    assert!(w[0].label.as_ref().unwrap().contains('2'));
    assert!(r.units[0].encrypted);
    assert_eq!(r.units[0].ports.len(), 2);
    assert_eq!(r.stopped_at, None);
    // an envelope before any module: a warning, nothing to record
    let r = run(
        "`pragma protect begin_protected\n`pragma protect key_block\nxxxx\n`pragma protect end_protected\n",
    );
    assert!(errors(&r).is_empty());
    assert_eq!(
        r.diagnostics.iter().filter(|d| d.code == "W4007").count(),
        1
    );
    assert!(r.units.is_empty());
    // an envelope inside the header: the module is recorded without ports
    let r = run(
        "`timescale 1ns/1ps\nmodule h #(\n`pragma protect begin_protected\nzz\n`pragma protect end_protected\n",
    );
    assert!(errors(&r).is_empty());
    assert_eq!(
        (
            r.units[0].name.as_str(),
            r.units[0].encrypted,
            r.units[0].ports.len()
        ),
        ("h", true, 0)
    );
    // two envelopes in one file: still one warning; the plain module before and after are units
    let r = run(
        "`timescale 1ns/1ps\nmodule a; endmodule\nmodule b (input x);\n`pragma protect begin_protected\nzz\n`pragma protect end_protected\nmodule c; endmodule\n",
    );
    assert_eq!(
        r.diagnostics.iter().filter(|d| d.code == "W4007").count(),
        1
    );
    assert_eq!(
        r.units
            .iter()
            .map(|u| (u.name.as_str(), u.encrypted))
            .collect::<Vec<_>>(),
        [("a", false), ("b", true), ("c", false)]
    );
    // the legacy forms hide everything up to the end marker
    let r = run(
        "`timescale 1ns/1ps\nmodule a; endmodule\n`protect\nmodule hidden; \"unterminated /*\nendmodule\n`endprotect\nmodule z; endmodule\n",
    );
    assert_eq!(
        r.units.iter().map(|u| u.name.as_str()).collect::<Vec<_>>(),
        ["a", "z"]
    );
    assert_eq!(
        r.diagnostics.iter().filter(|d| d.code == "W4007").count(),
        1
    );
    // an include that is encrypted: the module that includes it is recorded as encrypted
    let d = scratch();
    write(
        &d,
        "e.vh",
        "`pragma protect begin_protected\nzz\n`pragma protect end_protected\n",
    );
    let f = write(
        &d,
        "t.v",
        "`timescale 1ns/1ps\nmodule m (input a);\n`include \"e.vh\"\nendmodule\n",
    );
    let r = analyze(&[f], &Options::default());
    assert!(errors(&r).is_empty());
    assert!(r.units[0].encrypted);
    let w = r.diagnostics.iter().find(|d| d.code == "W4007").unwrap();
    assert!(r.files[w.span.unwrap().file].ends_with("e.vh"));
    // markers around plaintext warn once each
    let r = run(
        "`timescale 1ns/1ps\n`pragma protect begin\nmodule p; endmodule\n`pragma protect end\n",
    );
    assert_eq!(
        r.diagnostics.iter().map(|d| d.code).collect::<Vec<_>>(),
        ["W4009", "W4009"]
    );
    assert_eq!(r.units.len(), 1);
}

#[test]
fn systemverilog_is_named() {
    let d = scratch();
    let f = write(&d, "t.sv", "module m; endmodule\n");
    let r = analyze(std::slice::from_ref(&f), &Options::default());
    assert_eq!(r.diagnostics[0].code, "E4002");
    assert!(r.units.is_empty());
    let o = Options {
        lang_override: Some(Lang::Verilog2005),
        ..Options::default()
    };
    assert!(analyze(&[f], &o).units.len() == 1);
    // an odd extension needs --lang
    let g = write(&d, "t.vlog", "module m; endmodule\n");
    assert_eq!(
        analyze(std::slice::from_ref(&g), &Options::default()).diagnostics[0].code,
        "E4012"
    );
    assert_eq!(analyze(&[g], &o).units.len(), 1);

    let names = |src: &str| -> (String, usize) {
        let r = run(src);
        let d = r
            .diagnostics
            .iter()
            .find(|d| d.severity == Severity::Error)
            .expect("an error");
        assert_eq!(d.code, "E4003", "{}", d.message);
        (d.message.clone(), line(&r, d.span.unwrap()))
    };
    assert!(
        names("module m;\n logic [3:0] x;\nendmodule")
            .0
            .contains("`logic`")
    );
    assert_eq!(names("module m;\n logic [3:0] x;\nendmodule").1, 2);
    assert!(
        names("module m (input logic a);\nendmodule")
            .0
            .contains("logic")
    );
    assert!(
        names("module m;\n always_comb x = 1;\nendmodule")
            .0
            .contains("always_comb")
    );
    assert!(
        names("module m;\n initial begin\n i++;\n end\nendmodule")
            .0
            .contains("++")
    );
    assert!(
        names("module m;\n initial begin\n i += 1;\n end\nendmodule")
            .0
            .contains("+=")
    );
    assert!(
        names("module m;\n initial begin : b\n end : b\nendmodule")
            .0
            .contains("label")
    );
    assert!(names("module m;\nendmodule : m").0.contains("label"));
    assert!(
        names("module m;\n initial #10ns x = 1;\nendmodule")
            .0
            .contains("10ns")
    );
    assert!(names("module m;\n S u (.*);\nendmodule").0.contains(".*"));
    assert!(names("module m;\n S u (.a);\nendmodule").0.contains(".a"));
    assert!(names("package p;\nendpackage").0.contains("package"));
    assert!(names("module m;\n int x;\nendmodule").0.contains("`int`"));
    assert!(
        names(
            "module m;\n initial begin\n for (int i = 0; i < 2; i = i + 1) x = 1;\n end\nendmodule"
        )
        .0
        .contains("`int`")
    );
    assert!(
        names("module m;\n initial begin\n unique case (x) endcase\n end\nendmodule")
            .0
            .contains("unique")
    );
    assert!(
        names("module m;\n function logic f; endfunction\nendmodule")
            .0
            .contains("logic")
    );
    assert!(
        names("module m #(parameter int N = 1) ();\nendmodule")
            .0
            .contains("`int`")
    );
    assert!(
        names("`define F(a = 1) a\nmodule m; endmodule")
            .0
            .contains("default argument")
    );
    assert!(
        names("module m;\nwire x = `__LINE__;\nendmodule")
            .0
            .contains("__LINE__")
    );
}

#[test]
fn unsupported_constructs_are_named() {
    let r = bad("config c;\n design m;\nendconfig\n", "E4101", 1);
    assert!(r.diagnostics[0].message.contains("config"));
    // macromodule is another spelling of module (clause 12.1)
    let r = run("`timescale 1ns/1ps\nmacromodule m; endmodule\n");
    assert_eq!(errors(&r), []);
    assert_eq!(r.units.len(), 1);
}

#[test]
fn syntax_errors_say_what_was_expected_and_do_not_cascade() {
    let r = run(
        "module a;\n wire x = ;\n wire y = (1;\n assign z = ;\nendmodule\nmodule b;\n wire q = ;\nendmodule\nmodule c; endmodule\n",
    );
    // one error per broken module, then parsing resumes
    assert_eq!(errors(&r), [("E4100", 2), ("E4100", 7)]);
    assert!(
        r.diagnostics[0]
            .message
            .starts_with("expected an expression, found `;`")
    );
    assert!(r.units.is_empty(), "no unit from a failed file");
    bad("module m\n wire x;\nendmodule\n", "E4100", 2);
    bad("module m;\n wire x\nendmodule\n", "E4100", 3);
    bad("module m;\n", "E4100", 1);
    bad(
        "module m;\n initial begin\n x = 1;\nendmodule\n",
        "E4100",
        4,
    );
}

#[test]
fn analysis_errors() {
    bad("module a; endmodule\nmodule a; endmodule\n", "E4200", 2);
    let r = bad(
        "`default_nettype none\nmodule a (input x);\n assign y = x;\nendmodule\n",
        "E4201",
        3,
    );
    assert!(r.diagnostics[0].message.contains('y'));
    // declared names, hierarchical roots, labels and parameters are fine
    ok(
        "`default_nettype none\nmodule a #(parameter P = 1) (input x, output reg y);\n genvar g;\n wire w = glbl.GSR;\n always @(*) y = x & w & P;\n generate for (g=0; g<P; g=g+1) begin : b assign w = g; end endgenerate\n initial disable b;\nendmodule\n",
    );
    // a typo gets a suggestion
    let r = run("`default_nettype none\nmodule a (input clock);\n wire w = clok;\nendmodule\n");
    assert!(r.diagnostics[0].help.as_ref().unwrap().contains("clock"));
    // implicit nets are fine by default
    ok("module a (input x);\n assign y = x;\nendmodule\n");
    bad("module a;\n wire w;\n wire w;\nendmodule\n", "E4203", 3);
    // clause 12.3.4: a port of the header is not declared again in the body
    bad("module a (output o);\n wire o;\nendmodule\n", "E4203", 2);
    ok("module a (output o);\n reg r;\nendmodule\n");
}

#[test]
fn first_failing_file_stops_the_batch() {
    let d = scratch();
    let a = write(&d, "a.v", "module a; endmodule\n");
    let b = write(&d, "b.v", "module b; wire = ; endmodule\n");
    let c = write(&d, "c.v", "module c; endmodule\n");
    let r = analyze(&[a, b, c], &Options::default());
    assert_eq!(r.stopped_at, Some(1));
    assert_eq!(
        r.units.iter().map(|u| u.name.as_str()).collect::<Vec<_>>(),
        ["a"]
    );
}

#[test]
fn xil_timing_warns_that_checks_are_not_evaluated() {
    let src = "module m (input c, d);\nspecify\n $setup(d, posedge c, 1);\nendspecify\nendmodule\n";
    assert!(run(src).diagnostics.iter().all(|d| d.code != "W4015"));
    let mut o = Options::default();
    o.defines.push(("XIL_TIMING".into(), None));
    assert_eq!(
        run_with(src, &o)
            .diagnostics
            .iter()
            .filter(|d| d.code == "W4015")
            .count(),
        1
    );
}

#[test]
fn lexical_forms() {
    ok(r#"
module m;
  localparam a = 8'hFF, b = 'd10, c = 4'sb1x_z?, d = 16 'h dead_BEEF, e = 1_000, f = 12.5, g = 1e3, h = 2.5E-3, i = 'o17, j = "a \" b";
  wire \bus[3] , \a+b ;
  assign \bus[3]  = \a+b ;
  initial i = #0 'bz;
  wire [2:0] k = 3'b1 ?  3'b0 : 3'b1;
  wire l = ~&a | ^~b | ~|c;
  wire [3:0] n = a <<< 1 >>> 1 << 2 >> 3;
  wire o = a === b !== c == d != e <= f >= g;
  wire [7:0] p = x[3 +: 2] | x[7 -: 2] | x[1] | x[3:2];
  wire q = a -- b;
  wire r = a ++ b;
  initial @(* ) begin end
  initial @( *) begin end
endmodule
"#);
}

#[test]
fn bombs_and_deep_nesting_end_with_an_error() {
    // 40 levels of doubling: 2^40 copies without a cap
    let mut src = String::from("`define L0 x\n");
    for i in 1..40 {
        src += &format!("`define L{i} `L{} `L{}\n", i - 1, i - 1);
    }
    src += "module m; wire w = `L39; endmodule\n";
    let r = run(&src);
    assert!(r.diagnostics.iter().any(|d| d.code == "E4011"));
    // deep parentheses: fine up to the limit, an error beyond it
    let nest = |n: usize| {
        format!(
            "module m; wire w = {}1{}; endmodule\n",
            "(".repeat(n),
            ")".repeat(n)
        )
    };
    ok(&nest(60));
    let r = run(&nest(3000));
    assert!(
        r.diagnostics
            .iter()
            .any(|d| d.code == "E4100" && d.message.contains("too deep"))
    );
}

#[test]
fn a_macro_named_like_an_ignored_directive_is_a_macro() {
    // `accelerate is ignored with W4016 only while no macro of that name is defined.
    ok("`define accelerate 8\nmodule m; wire [`accelerate-1:0] w; endmodule\n");
    let r = run("`accelerate\nmodule m; endmodule\n");
    assert!(r.diagnostics.iter().any(|d| d.code == "W4016"));
}

#[test]
fn a_long_chain_of_conditionals_is_not_nesting() {
    // Clause 5.1.13: `c1 ? a1 : c2 ? a2 : ... : e` groups to the right; a table of 1000 rows is legal.
    let mut e = String::new();
    for i in 0..1000 {
        e += &format!("s == {i} ? {i} : ");
    }
    ok(&format!(
        "module m; wire [31:0] s; wire [31:0] w = {e}0; endmodule\n"
    ));
    assert_eq!(
        shape(&expr_of("a ? b : c ? d : e")),
        "(a ? b : (c ? d : e))"
    );
}

fn expr_of(text: &str) -> yavs_verilog::ast::Expr {
    use yavs_verilog::ast::{Decl, Item};
    let src = format!("module m; wire w = {text}; endmodule");
    let Ok(d) = yavs_verilog::parse_str(&src, &Options::default()) else {
        panic!("{text} does not parse")
    };
    let Decl::Module(m) = &d[0] else { panic!() };
    let Item::Net(n) = &m.items[0] else { panic!() };
    n.names[0].init.clone().unwrap()
}

/// The shape of an expression as a fully parenthesised string.
fn shape(e: &yavs_verilog::ast::Expr) -> String {
    use yavs_verilog::ast::Expr::*;
    match e {
        Num(s) | Real(s) => s.clone(),
        Ident(i) => i.name.clone(),
        Unary(o, x) => format!("({o} {})", shape(x)),
        Binary(o, a, b) => format!("({} {o} {})", shape(a), shape(b)),
        Cond(c, a, b) => format!("({} ? {} : {})", shape(c), shape(a), shape(b)),
        Concat(v) => format!("{{{}}}", v.iter().map(shape).collect::<Vec<_>>().join(",")),
        Repl(n, v) => format!(
            "{{{}{{{}}}}}",
            shape(n),
            v.iter().map(shape).collect::<Vec<_>>().join(",")
        ),
        Index(a, i) => format!("{}[{}]", shape(a), shape(i)),
        Range(a, k, x, y) => format!("{}[{} {k:?} {}]", shape(a), shape(x), shape(y)),
        Dot(a, i) => format!("{}.{}", shape(a), i.name),
        Call(n, a) => format!(
            "{}({})",
            shape(n),
            a.iter().map(shape).collect::<Vec<_>>().join(",")
        ),
        SysCall(n, a) => format!("{n}({})", a.iter().map(shape).collect::<Vec<_>>().join(",")),
        MinTypMax(a, b, c) => format!("{}:{}:{}", shape(a), shape(b), shape(c)),
        Str(s) => format!("{s:?}"),
        Empty => "_".into(),
    }
}

#[test]
fn operator_precedence_and_associativity() {
    let s = |t: &str| shape(&expr_of(t));
    assert_eq!(s("a + b * c"), "(a + (b * c))");
    assert_eq!(s("a - b - c"), "((a - b) - c)");
    assert_eq!(s("a ** b ** c"), "((a ** b) ** c)");
    assert_eq!(s("a | b & c ^ d"), "(a | ((b & c) ^ d))");
    assert_eq!(s("a || b && c"), "(a || (b && c))");
    assert_eq!(s("a == b < c"), "(a == (b < c))");
    assert_eq!(s("a << b + c"), "(a << (b + c))");
    assert_eq!(s("a ? b : c ? d : e"), "(a ? b : (c ? d : e))");
    assert_eq!(s("a ? b ? c : d : e"), "(a ? (b ? c : d) : e)");
    assert_eq!(s("-a ** b"), "((- a) ** b)");
    assert_eq!(s("~&a | ^b"), "((~& a) | (^ b))");
    assert_eq!(s("!a && ~b"), "((! a) && (~ b))");
    assert_eq!(s("{a, b[1], {2{c}}}"), "{a,b[1],{2{c}}}");
    assert_eq!(
        s("a[3:0] + b[i +: 2] - c[j -: 2]"),
        "((a[3 Colon 0] + b[i PlusColon 2]) - c[j MinusColon 2])"
    );
    assert_eq!(
        s("top.u0.x[2] & f(a, b) & $clog2(n)"),
        "((top.u0.x[2] & f(a,b)) & $clog2(n))"
    );
    assert_eq!(s("a --b"), "(a - (- b))");
    assert_eq!(s("a<= b"), "(a <= b)");
    assert_eq!(s("(1:2:3)"), "1:2:3");
    assert_eq!(s("a + (* keep *) b"), "(a + b)");
}

#[test]
fn statements_and_instances_have_the_expected_shape() {
    use yavs_verilog::ast::*;
    let d = yavs_verilog::parse_str(
        "module m (a, b); input a; output b; reg b;\n always @(posedge a or negedge b) b <= #2 a ? 1 : 0;\n always @* if (a) b = 1; else b = 0;\n Sub #(3) u (a, , b); endmodule",
        &Options::default(),
    )
    .unwrap();
    let Decl::Module(m) = &d[0] else { panic!() };
    let Item::Always(Stmt::Event {
        ctrl: EventCtrl::List(l),
        body,
    }) = &m.items[3]
    else {
        panic!("{:?}", m.items[3])
    };
    assert_eq!((l[0].edge, l[1].edge), (Some("posedge"), Some("negedge")));
    let Stmt::Assign {
        nonblocking,
        ctrl: Some(Ctrl::Delay(_)),
        ..
    } = &**body
    else {
        panic!()
    };
    assert!(*nonblocking);
    let Item::Always(Stmt::Event {
        ctrl: EventCtrl::Star,
        body,
    }) = &m.items[4]
    else {
        panic!()
    };
    assert!(matches!(&**body, Stmt::If { els: Some(_), .. }));
    let Item::Inst(i) = &m.items[5] else { panic!() };
    assert_eq!(
        (
            i.module.name.as_str(),
            i.params.len(),
            i.insts[0].conns.len()
        ),
        ("Sub", 1, 3)
    );
    assert!(matches!(&i.insts[0].conns[1], Conn::Ordered(Expr::Empty)));
}

#[test]
fn udp_rows_are_decoded() {
    use yavs_verilog::ast::*;
    let d = yavs_verilog::parse_str(
        "primitive p (q, d, c); output q; reg q; input d, c; initial q = 0; table 0 (01) : ? : 0; 1 (x1) : 1 : -; endtable endprimitive",
        &Options::default(),
    )
    .unwrap();
    let Decl::Udp(u) = &d[0] else { panic!() };
    assert!(u.sequential && u.init.is_some());
    assert_eq!(u.rows.len(), 2);
    assert_eq!(
        u.rows[0].inputs,
        [UdpSym::Level('0'), UdpSym::Edge('0', '1')]
    );
    assert_eq!(u.rows[0].state, Some(UdpSym::Level('?')));
    assert_eq!(u.rows[1].inputs[1], UdpSym::Edge('x', '1'));
    assert_eq!(u.rows[1].output, UdpSym::Level('-'));
    let e = yavs_verilog::parse_str(
        "primitive p (q, d); output q; input d; table 0 1 : 1; endtable endprimitive",
        &Options::default(),
    )
    .unwrap_err();
    assert_eq!(e[0].code, "E4100");
}

#[test]
fn context_digest_follows_everything_that_changes_a_unit() {
    let d = scratch();
    let f = write(&d, "t.v", "module m; endmodule\n");
    let base = analyze(std::slice::from_ref(&f), &Options::default()).units[0]
        .context_digest
        .clone();
    let again = analyze(std::slice::from_ref(&f), &Options::default()).units[0]
        .context_digest
        .clone();
    assert_eq!(base, again);
    let ts = Options {
        default_timescale: Timescale {
            unit_fs: 1_000,
            precision_fs: 1_000,
        },
        ..Options::default()
    };
    assert_ne!(
        base,
        analyze(std::slice::from_ref(&f), &ts).units[0].context_digest
    );
    let inc = Options {
        include_dirs: vec![d.clone()],
        ..Options::default()
    };
    assert_ne!(base, analyze(&[f], &inc).units[0].context_digest);
}

#[test]
fn timing_check_warning_is_once_per_call() {
    let d = scratch();
    let src = "module m (input c, d);\nspecify\n $setup(d, posedge c, 1);\nendspecify\nendmodule\n";
    let a = write(&d, "a.v", src);
    let b = write(&d, "b.v", &src.replace("module m", "module n"));
    let o = Options {
        defines: vec![("XIL_TIMING".into(), None)],
        ..Options::default()
    };
    let r = analyze(&[a, b], &o);
    assert_eq!(
        r.diagnostics.iter().filter(|d| d.code == "W4015").count(),
        1
    );
}

#[test]
fn plaintext_after_an_envelope_belongs_to_the_module() {
    let r = run(
        "`timescale 1ns/1ps\nmodule a (input x);\n`pragma protect begin_protected\nzz\n`pragma protect end_protected\n  wire w;\n  assign w = x;\nendmodule\nmodule b; endmodule\n",
    );
    assert!(errors(&r).is_empty(), "{:?}", r.diagnostics);
    assert_eq!(
        r.units
            .iter()
            .map(|u| (u.name.as_str(), u.encrypted))
            .collect::<Vec<_>>(),
        [("a", true), ("b", false)]
    );
}

#[test]
fn unreadable_files_have_no_span() {
    let r = analyze(&[PathBuf::from("/nonexistent/x.v")], &Options::default());
    assert_eq!(
        (r.diagnostics[0].code, r.diagnostics[0].span.is_none()),
        ("E4012", true)
    );
    let d = run("`define A\n## x\n");
    assert_eq!(d.diagnostics[0].code, "E4100");
}

#[test]
fn diagnostics_are_owned_by_the_argument_file_that_found_them() {
    let d = scratch();
    let inc = write(&d, "bad.vh", "`ifdef X\n");
    let a = write(&d, "a.v", "module a; endmodule\n");
    let b = write(
        &d,
        "b.v",
        &format!("`include \"{}\"\nmodule b; endmodule\n", inc.display()),
    );
    let r = analyze(&[a, b], &Options::default());
    assert_eq!(r.diag_file.len(), r.diagnostics.len());
    let in_include = r
        .diagnostics
        .iter()
        .position(|d| d.code == "E4004")
        .unwrap();
    assert_eq!(r.diag_file[in_include], 1);
    assert!(r.diag_file.windows(2).all(|w| w[0] <= w[1]));
}

#[test]
fn elaboration_tasks_are_items_at_module_level_and_in_generate_blocks() {
    let r = ok(
        "module m #(parameter W = 8);\n genvar i;\n $info(\"w=%0d\", W);\n $fatal;\n generate\n  if (W < 2) begin : a\n   $error(\"too small\");\n  end else $warning();\n  for (i = 0; i < 2; i = i + 1) begin : g\n   $info(\"i=%0d\", i);\n  end\n endgenerate\nendmodule\n",
    );
    assert_eq!(r.units.len(), 1);
}

#[test]
fn other_system_tasks_are_not_module_items() {
    bad("module m;\n $display(\"x\");\nendmodule\n", "E4100", 2);
    bad("module m;\n $error(;\nendmodule\n", "E4100", 2);
}
