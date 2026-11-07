//! VHDL testbenches that instantiate Verilog modules, through the command line (#776 step C):
//! `yavs analyze` of the `.v` and `.vhd` files, then `yavs run`. Each testbench checks its own
//! results; the tests compare the reports, with their time and delta.

mod common;
use common::{Tmp, tempdir};

use std::process::{Command, Output};

struct T(Tmp);

impl T {
    fn new(name: &str) -> T {
        T(tempdir(name))
    }

    fn write(&self, name: &str, text: &str) -> &Self {
        std::fs::write(self.0.join(name), text).unwrap();
        self
    }

    fn yavs(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_yavs"))
            .current_dir(&*self.0)
            .args(args)
            .output()
            .unwrap()
    }

    fn analyze(&self, args: &[&str]) -> &Self {
        let o = self.yavs(&[&["analyze", "--work=work:lib"], args].concat());
        assert!(o.status.success(), "analyze {args:?}:\n{}", err(&o));
        self
    }

    fn run(&self, args: &[&str]) -> Output {
        self.yavs(&[&["run", "--work=work:lib"], args].concat())
    }

    /// The report lines of a successful run: `TIME+DELTA: message`.
    fn notes(&self, args: &[&str]) -> Vec<String> {
        let o = self.run(args);
        assert!(
            o.status.success(),
            "run {args:?}:\n{}\n{}",
            out(&o),
            err(&o)
        );
        notes(&o)
    }
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn notes(o: &Output) -> Vec<String> {
    out(o)
        .lines()
        .filter_map(|l| l.strip_prefix("** Note: "))
        .map(String::from)
        .collect()
}

const COUNTER: &str = "`timescale 1ns/1ps
module counter #(parameter WIDTH = 4) (input clk, input rst, output reg [WIDTH-1:0] q);
  always @(posedge clk or posedge rst)
    if (rst) q <= 0;
    else q <= q + 1;
endmodule
";

const COUNTER_TB: &str = "library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
  component counter is
    generic (WIDTH : integer := 4);
    port (clk, rst : in std_logic; q : out std_logic_vector(WIDTH - 1 downto 0));
  end component;
  signal clk, rst : std_logic := '0';
  signal q : std_logic_vector(7 downto 0);
begin
  dut : counter generic map (WIDTH => 8) port map (clk => clk, rst => rst, q => q);
  process
  begin
    wait on q;
    report \"first \" & to_string(q);
    wait;
  end process;
  process
  begin
    rst <= '1'; wait for 5 ns; rst <= '0';
    for i in 1 to 3 loop
      clk <= '1'; wait for 5 ns; clk <= '0'; wait for 5 ns;
      report \"q \" & integer'image(to_integer(unsigned(q)));
    end loop;
    assert unsigned(q) = 3 severity failure;
    wait;
  end process;
end architecture;
";

/// A register, a reset and an adder of a Verilog module, driven and read by VHDL; the output
/// starts at x one delta into the run, the module's initial value (D4).
#[test]
fn counter() {
    let t = T::new("v-counter");
    t.write("counter.v", COUNTER)
        .write("tb.vhd", COUNTER_TB)
        .analyze(&["counter.v", "tb.vhd"]);
    assert_eq!(
        t.notes(&["tb"]),
        [
            "0fs+1: first XXXXXXXX",
            "15000000fs+0: q 1",
            "25000000fs+0: q 2",
            "35000000fs+0: q 3"
        ]
    );
    // The output does not depend on the number of threads.
    let one = out(&t.run(&["tb"]));
    assert_eq!(out(&t.run(&["--threads", "4", "tb"])), one);
}

/// Shifts and an adder as continuous assignments: an x or a U bit (read as x) from VHDL makes
/// the sum x, a shift amount with one makes the shift x.
#[test]
fn shifter_and_adder_with_x() {
    let t = T::new("v-comb");
    t.write(
        "comb.v",
        "`timescale 1ns/1ps
module comb(input [7:0] a, input [7:0] b, input [2:0] s,
            output [8:0] sum, output [7:0] shl, output [7:0] sar);
  assign sum = a + b;
  assign shl = a << s;
  assign sar = $signed(a) >>> s;
endmodule
",
    )
    .write(
        "tb.vhd",
        "library ieee;
use ieee.std_logic_1164.all;
entity tb is end entity;
architecture sim of tb is
  component comb is
    port (a, b : in std_logic_vector(7 downto 0); s : in std_logic_vector(2 downto 0);
          sum : out std_logic_vector(8 downto 0); shl, sar : out std_logic_vector(7 downto 0));
  end component;
  signal a, b : std_logic_vector(7 downto 0);
  signal s : std_logic_vector(2 downto 0);
  signal sum : std_logic_vector(8 downto 0);
  signal shl, sar : std_logic_vector(7 downto 0);
begin
  dut : comb port map (a, b, s, sum, shl, sar);
  process
    procedure show is
    begin
      report to_string(sum) & \" \" & to_string(shl) & \" \" & to_string(sar);
    end procedure;
  begin
    a <= \"10000001\"; b <= \"00000011\"; s <= \"001\";
    wait for 1 ns; show;
    b <= \"0000001X\";
    wait for 1 ns; show;
    b <= \"0000001L\"; s <= \"0U1\";
    wait for 1 ns; show;
    wait;
  end process;
end architecture;
",
    )
    .analyze(&["comb.v", "tb.vhd"]);
    assert_eq!(
        t.notes(&["tb"]),
        [
            "1000000fs+0: 010000100 00000010 11000000",
            "2000000fs+0: XXXXXXXXX 00000010 11000000",
            "3000000fs+0: 010000011 XXXXXXXX XXXXXXXX"
        ]
    );
}

/// A Moore detector of 101 with localparam states and a case statement.
#[test]
fn fsm() {
    let t = T::new("v-fsm");
    t.write(
        "det.v",
        "`timescale 1ns/1ps
module det(input clk, input rst, input din, output reg found);
  localparam S0 = 2'd0, S1 = 2'd1, S10 = 2'd2;
  reg [1:0] st;
  always @(posedge clk) begin
    if (rst) begin
      st <= S0;
      found <= 1'b0;
    end else begin
      found <= 1'b0;
      case (st)
        S0: if (din) st <= S1;
        S1: if (!din) st <= S10;
        S10: if (din) begin st <= S1; found <= 1'b1; end else st <= S0;
        default: st <= S0;
      endcase
    end
  end
endmodule
",
    )
    .write(
        "tb.vhd",
        "library ieee;
use ieee.std_logic_1164.all;
entity tb is end entity;
architecture sim of tb is
  component det is
    port (clk, rst, din : in std_logic; found : out std_logic);
  end component;
  signal clk, rst, din, found : std_logic := '0';
  constant bits : std_logic_vector(1 to 8) := \"10101101\";
begin
  dut : det port map (clk, rst, din, found);
  process
    variable n : natural := 0;
  begin
    rst <= '1';
    clk <= '1'; wait for 5 ns; clk <= '0'; wait for 5 ns;
    rst <= '0';
    for i in bits'range loop
      din <= bits(i);
      clk <= '1'; wait for 5 ns; clk <= '0'; wait for 5 ns;
      if found = '1' then
        report \"found after bit \" & integer'image(i);
        n := n + 1;
      end if;
    end loop;
    assert n = 3 severity failure;
    wait;
  end process;
end architecture;
",
    )
    .analyze(&["det.v", "tb.vhd"]);
    assert_eq!(
        t.notes(&["tb"]),
        [
            "40000000fs+0: found after bit 3",
            "60000000fs+0: found after bit 5",
            "90000000fs+0: found after bit 8"
        ]
    );
}

/// Widths from a generic, a string parameter that selects behaviour, a vector parameter with a
/// declared range from a std_logic_vector generic.
#[test]
fn parameterized_widths_and_string_parameter() {
    let t = T::new("v-param");
    t.write(
        "pw.v",
        "`timescale 1ns/1ps
module pw #(parameter W = 4, parameter MODE = \"ADD\", parameter [W-1:0] INIT = 0)
           (input [W-1:0] a, input [W-1:0] b, output [W-1:0] y);
  assign y = (MODE == \"ADD\") ? a + b + INIT : a - b;
endmodule
",
    )
    .write(
        "tb.vhd",
        "library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
  component pw is
    generic (W : integer := 4; MODE : string := \"ADD\"; INIT : std_logic_vector(11 downto 0) := (others => '0'));
    port (a, b : in std_logic_vector(W - 1 downto 0); y : out std_logic_vector(W - 1 downto 0));
  end component;
  signal a, b, y1, y2 : std_logic_vector(11 downto 0);
begin
  add : pw generic map (W => 12, MODE => \"ADD\", INIT => x\"001\") port map (a, b, y1);
  sub : pw generic map (W => 12, MODE => \"SUB\") port map (a, b, y2);
  process
  begin
    a <= std_logic_vector(to_unsigned(1000, 12));
    b <= std_logic_vector(to_unsigned(24, 12));
    wait for 1 ns;
    report integer'image(to_integer(unsigned(y1))) & \" \" & integer'image(to_integer(unsigned(y2)));
    wait;
  end process;
end architecture;
",
    )
    .analyze(&["pw.v", "tb.vhd"]);
    assert_eq!(t.notes(&["tb"]), ["1000000fs+0: 1025 976"]);
}

/// A clock and a register made in Verilog, a VHDL flip-flop on that clock: the clock reaches
/// VHDL one delta after the domain ran (Active region) and the register one more delta later
/// (NBA region), so the VHDL flip-flop samples the value before the edge (D4).
#[test]
fn verilog_clock_into_vhdl_flop() {
    let t = T::new("v-clock");
    t.write(
        "clkgen.v",
        "`timescale 1ns/1ps
module clkgen(output reg clk, output reg [3:0] q);
  initial begin
    clk = 0;
    q = 0;
  end
  always #5 clk = ~clk;
  always @(posedge clk) q <= q + 1;
endmodule
",
    )
    .write(
        "tb.vhd",
        "library ieee;
use ieee.std_logic_1164.all;
entity tb is end entity;
architecture sim of tb is
  component clkgen is
    port (clk : out std_logic; q : out std_logic_vector(3 downto 0));
  end component;
  signal clk : std_logic;
  signal q, sample : std_logic_vector(3 downto 0);
begin
  gen : clkgen port map (clk, q);
  flop : process (clk)
  begin
    if rising_edge(clk) then
      sample <= q;
    end if;
  end process;
  process
  begin
    wait on clk;
    report \"clk \" & to_string(clk);
  end process;
  process
  begin
    wait on q;
    report \"q \" & to_string(q);
  end process;
  process
  begin
    wait on sample;
    report \"sample \" & to_string(sample);
  end process;
end architecture;
",
    )
    .analyze(&["clkgen.v", "tb.vhd"]);
    let want = [
        "0fs+1: clk 0",
        "0fs+1: q 0000",
        "5000000fs+1: clk 1",
        "5000000fs+2: q 0001",
        "5000000fs+2: sample 0000",
        "10000000fs+1: clk 0",
        "15000000fs+1: clk 1",
        "15000000fs+2: q 0010",
        "15000000fs+2: sample 0001",
    ];
    assert_eq!(t.notes(&["--stop-time=16ns", "tb"]), want);
    assert_eq!(t.notes(&["--stop-time=16ns", "--threads", "4", "tb"]), want);
}

/// Delays in the time unit of each module, rounded to its precision (19.8); a module without
/// a `timescale` takes the default 1ns/1ps.
#[test]
fn timescale_scaling() {
    let t = T::new("v-ts");
    t.write(
        "ts.v",
        "`timescale 1ns/100ps
module ts_a(output reg o);
  initial begin o = 0; #1.55 o = 1; end
endmodule
`timescale 1ps/1ps
module ts_b(output reg o);
  initial begin o = 0; #1500 o = 1; end
endmodule
",
    )
    .write(
        "tb.vhd",
        "library ieee;
use ieee.std_logic_1164.all;
entity tb is end entity;
architecture sim of tb is
  component ts_a is port (o : out std_logic); end component;
  component ts_b is port (o : out std_logic); end component;
  signal a, b : std_logic;
begin
  ua : ts_a port map (a);
  ub : ts_b port map (b);
  process begin wait until a = '1'; report \"a \" & time'image(now); wait; end process;
  process begin wait until b = '1'; report \"b \" & time'image(now); wait; end process;
end architecture;
",
    )
    .analyze(&["ts.v", "tb.vhd"]);
    assert_eq!(
        t.notes(&["tb"]),
        ["1500000fs+1: b 1500000 fs", "1600000fs+1: a 1600000 fs"]
    );
}

/// The ports of a component bound to a Verilog module are signals of the wave file, under the
/// instance.
#[test]
fn waves_show_the_verilog_ports() {
    let t = T::new("v-wave");
    t.write("counter.v", COUNTER)
        .write("tb.vhd", COUNTER_TB)
        .analyze(&["counter.v", "tb.vhd"]);
    let o = t.run(&["--wave=w.fst", "tb"]);
    assert!(o.status.success(), "{}", err(&o));
    let mut w = wellen::simple::read(t.0.join("w.fst")).unwrap();
    let h = w.hierarchy();
    let names: Vec<String> = h.all_vars().map(|r| h[r].full_name(h)).collect();
    for want in ["tb.dut.clk", "tb.dut.rst", "tb.dut.q"] {
        assert!(names.iter().any(|n| n == want), "{want} not in {names:?}");
    }
    let q = h
        .all_vars()
        .find(|&r| h[r].full_name(h) == "tb.dut.q")
        .unwrap();
    let sig = h[q].signal_ref();
    w.load_signals(&[sig]);
    let changes = w.get_signal(sig).unwrap().iter_changes().count();
    assert!(changes >= 4, "{changes} changes of tb.dut.q");
}

/// The search for a module of a component that no VHDL entity binds: the work library, then
/// the `-L` libraries in order; without it the component stays unbound, and so does one
/// configured `use open`.
#[test]
fn library_search_and_explicit_open() {
    let t = T::new("v-libs");
    t.write("counter.v", COUNTER).write("tb.vhd", COUNTER_TB);
    let o = t.yavs(&["analyze", "--work=vlib:vl", "counter.v"]);
    assert!(o.status.success(), "{}", err(&o));
    t.analyze(&["tb.vhd"]);
    let o = t.run(&["--map=vlib:vl", "-L", "vlib", "tb"]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("q 3"), "{}", out(&o));
    // Not searched: unbound, and the testbench's assertion fails.
    let o = t.run(&["--map=vlib:vl", "tb"]);
    assert!(err(&o).contains("W2108"), "{}", err(&o));
    let o = t.run(&["-L", "nolib", "tb"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(err(&o).contains("E0142"), "{}", err(&o));
    // `use open` in the work library that holds the module.
    t.analyze(&["counter.v"]).write(
        "open.vhd",
        &COUNTER_TB
            .replace("entity tb", "entity tbo")
            .replace("of tb is", "of tbo is")
            .replace(
                "begin\n  dut",
                "  for dut : counter use open;\nbegin\n  dut",
            ),
    );
    t.analyze(&["open.vhd"]);
    let o = t.run(&["tbo"]);
    assert!(err(&o).contains("W2108"), "{}", err(&o));
}

/// Elaboration reads a file as its analysis did: macros of an earlier file of the same
/// `analyze` call and `-D` definitions; a file changed since then is `E4508`.
#[test]
fn files_are_read_again_as_analyzed() {
    let t = T::new("v-setup");
    t.write("defs.v", "`define STEP 2\n")
        .write(
            "c.v",
            "`timescale 1ns/1ps
module counter #(parameter WIDTH = 4) (input clk, input rst, output reg [WIDTH-1:0] q);
`ifdef ONE
  always @(posedge clk or posedge rst) q <= rst ? 0 : q + 1;
`else
  always @(posedge clk or posedge rst) q <= rst ? 0 : q + `STEP;
`endif
endmodule
",
        )
        .write("tb.vhd", COUNTER_TB)
        .analyze(&["defs.v", "c.v", "tb.vhd"]);
    let o = t.run(&["tb"]);
    // Steps of 2: the testbench's check of 3 fails at the end.
    assert!(
        out(&o).contains("q 2") && out(&o).contains("q 6"),
        "{}",
        out(&o)
    );
    t.analyze(&["-D", "ONE", "c.v"]);
    let o = t.run(&["tb"]);
    assert!(o.status.success(), "{}", err(&o));
    std::fs::write(t.0.join("c.v"), COUNTER.replace("q + 1", "q + 3")).unwrap();
    let o = t.run(&["tb"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(err(&o).contains("E4508"), "{}", err(&o));
}

/// What does not fit between the component and the module, and what Verilog elaboration does
/// not implement, stops the run before it starts, naming it.
#[test]
fn mismatches_and_unsupported_constructs_fail_early() {
    let cases = [
        // A generic for a local parameter, and one with no parameter.
        (
            "module m #(parameter A = 1) (input i, output o); localparam B = 2; assign o = i; endmodule",
            "generic (A : integer := 1; B : integer := 2); port (i : in std_logic; o : out std_logic);",
            "E4701",
        ),
        (
            "module m (input i, output o); assign o = i; endmodule",
            "generic (WIDTH : integer := 1); port (i : in std_logic; o : out std_logic);",
            "E4701",
        ),
        // A port missing in the module, a width that differs, a direction that differs.
        (
            "module m (input i, output o); assign o = i; endmodule",
            "port (i, j : in std_logic; o : out std_logic);",
            "E4700",
        ),
        (
            "module m (input [3:0] i, output o); assign o = i[0]; endmodule",
            "port (i : in std_logic; o : out std_logic);",
            "E4700",
        ),
        (
            "module m (input i, output o); assign o = i; endmodule",
            "port (i : out std_logic; o : out std_logic);",
            "E4700",
        ),
        // A construct that is not implemented.
        (
            "module m (input i, output o); genvar g; generate for (g = 0; g < 1; g = g + 1) begin : b assign o = i; end endgenerate endmodule",
            "port (i : in std_logic; o : out std_logic);",
            "E4501",
        ),
        // A uwire driven twice.
        (
            "module m (input i, output o); uwire w; assign w = i; assign w = ~i; assign o = w; endmodule",
            "port (i : in std_logic; o : out std_logic);",
            "E4502",
        ),
        // An input that the module drives too, and a pulled input: the resolution with VHDL is
        // the inout of a later step.
        (
            "module m (input i, output o); assign i = 1'b0; assign o = i; endmodule",
            "port (i : in std_logic; o : out std_logic);",
            "E4501",
        ),
        (
            "module m (input tri1 i, output o); assign o = i; endmodule",
            "port (i : in std_logic; o : out std_logic);",
            "E4501",
        ),
        // A wired net.
        (
            "module m (input i, output o); wand w; assign w = i; assign o = w; endmodule",
            "port (i : in std_logic; o : out std_logic);",
            "E4501",
        ),
    ];
    for (k, (v, comp, code)) in cases.iter().enumerate() {
        let t = T::new("v-bad");
        let comp = comp.replace("in std_logic;", "in std_logic := '0';");
        t.write("m.v", &format!("`timescale 1ns/1ps\n{v}\n")).write(
            "tb.vhd",
            &format!(
                "library ieee; use ieee.std_logic_1164.all;
entity tb is end entity;
architecture sim of tb is
  component m is {comp} end component;
begin
  u : m;
end architecture;
"
            ),
        );
        // `u : m;` leaves every port open: the binding is what is checked.
        let o = t.yavs(&["analyze", "--work=work:lib", "m.v", "tb.vhd"]);
        if !o.status.success() {
            panic!("case {k}: analyze: {}", err(&o));
        }
        let o = t.run(&["tb"]);
        assert_eq!(o.status.code(), Some(2), "case {k}: {}", err(&o));
        assert!(
            err(&o).contains(code),
            "case {k}: expected {code}:\n{}",
            err(&o)
        );
        // `elab` stops at the same place.
        let o = t.yavs(&["elab", "--work=work:lib", "tb"]);
        assert_eq!(o.status.code(), Some(2), "case {k}: {}", err(&o));
        assert!(err(&o).contains(code), "case {k}: elab: {}", err(&o));
    }
}

/// Generics of every type a parameter takes, also a vector wider than 64 bits, and `%m`: the
/// path of the instance; `$finish` ends the run with status 0 before the testbench's later report.
#[test]
fn generic_types_display_and_finish() {
    let t = T::new("v-gtypes");
    t.write(
        "g.v",
        "`timescale 1ns/1ps
module g #(parameter I = 0, parameter R = 0.0, parameter S = \"\", parameter B = 0,
           parameter BOOL = 0, parameter SL = 0, parameter BV = 0, parameter [255:0] INIT = 0) ();
  initial begin
    $display(\"%m: %0d %f %s %b %b %b %b\", I, R, S, B, BOOL, SL, BV);
    $display(\"%h\", INIT);
    #3 $finish;
  end
endmodule
",
    )
    .write(
        "tb.vhd",
        "library ieee;
use ieee.std_logic_1164.all;
entity tb is end entity;
architecture sim of tb is
  component g is
    generic (I : integer; R : real; S : string; B : bit; BOOL : boolean; SL : std_logic;
             BV : bit_vector(3 downto 0); INIT : std_logic_vector(255 downto 0));
  end component;
begin
  u : g generic map (I => -7, R => 2.5, S => \"abc\", B => '1', BOOL => true, SL => 'Z',
                     BV => \"1010\", INIT => (255 => '1', 0 => '1', others => '0'));
  process
  begin
    report \"before\";
    wait for 10 ns;
    report \"not reached\";
    wait;
  end process;
end architecture;
",
    )
    .analyze(&["g.v", "tb.vhd"]);
    let o = t.run(&["tb"]);
    assert!(o.status.success(), "{}", err(&o));
    let text = out(&o);
    assert!(
        text.contains("tb.u: -7 2.500000 abc 1 1 z 1010\n"),
        "{text}"
    );
    let hex = format!("8{}1", "0".repeat(62));
    assert!(text.contains(&format!("{hex}\n")), "{text}");
    assert!(
        text.contains("before") && !text.contains("not reached"),
        "{text}"
    );
    // `elab` elaborates the Verilog too, and stops at what does not fit.
    let o = t.yavs(&["elab", "--work=work:lib", "tb"]);
    assert!(o.status.success(), "{}", err(&o));
}

/// `bit` and `bit_vector` ports: x and z from Verilog arrive as '0'; a VHDL vector of
/// ascending range maps left to left onto a descending Verilog vector.
#[test]
fn bit_ports_and_ascending_ranges() {
    let t = T::new("v-bits");
    t.write(
        "bp.v",
        "`timescale 1ns/1ps
module bp(input [3:0] a, output [3:0] y, output z, output reg r,
          input [7:0] w, output [0:7] v, output lsb);
  assign y = a + 1;
  assign v = w;
  assign lsb = w[0];
endmodule
",
    )
    .write(
        "tb.vhd",
        "library ieee;
use ieee.std_logic_1164.all;
entity tb is end entity;
architecture sim of tb is
  component bp is
    port (a : in bit_vector(3 downto 0); y : out bit_vector(3 downto 0); z, r : out bit;
          w : in std_logic_vector(0 to 7); v : out std_logic_vector(7 downto 0); lsb : out std_logic);
  end component;
  signal a, y : bit_vector(3 downto 0);
  signal z, r : bit;
  signal w : std_logic_vector(0 to 7);
  signal v : std_logic_vector(7 downto 0);
  signal lsb : std_logic;
begin
  u : bp port map (a, y, z, r, w, v, lsb);
  process
  begin
    a <= \"0110\";
    w <= \"10000011\";
    wait for 1 ns;
    report to_string(y) & \" \" & to_string(z) & \" \" & to_string(r) & \" \" & to_string(v) & \" \" & to_string(lsb);
    wait;
  end process;
end architecture;
",
    )
    .analyze(&["bp.v", "tb.vhd"]);
    assert_eq!(t.notes(&["tb"]), ["1000000fs+0: 0111 0 0 10000011 1"]);
}

/// A zero-delay loop between two Verilog processes ends the run with the domain's event limit,
/// as a run-time error that names the process, not a crash.
#[test]
fn a_zero_delay_loop_is_a_runtime_error() {
    let t = T::new("v-loop");
    t.write(
        "l.v",
        "`timescale 1ns/1ps
module l(output reg a);
  reg b;
  always @(a) b = ~b;
  always @(b) a = ~a;
  initial begin a = 0; b = 0; end
endmodule
",
    )
    .write(
        "tb.vhd",
        "library ieee;
use ieee.std_logic_1164.all;
entity tb is end entity;
architecture sim of tb is
  component l is port (a : out std_logic); end component;
  signal a : std_logic;
begin
  u : l port map (a);
end architecture;
",
    )
    .analyze(&["l.v", "tb.vhd"]);
    let o = t.run(&["tb"]);
    let e = err(&o);
    assert_eq!(o.status.code(), Some(1), "{e}");
    assert!(e.contains("E3009") && e.contains("tb.u.always"), "{e}");
    assert!(!e.contains("E9999") && !e.contains("panicked"), "{e}");
}

/// An encrypted module is recorded at analysis and refused when VHDL binds to it.
#[test]
fn an_encrypted_module_is_refused_when_bound() {
    let t = T::new("v-enc");
    t.write(
        "enc.v",
        "`timescale 1ns/1ps
module enc (input a, output y);
`pragma protect begin_protected
`pragma protect version = 1
`pragma protect data_block
dGhpcyBpcyBub3QgdmVyaWxvZw==
`pragma protect end_protected
endmodule
",
    )
    .write(
        "tb.vhd",
        "library ieee;
use ieee.std_logic_1164.all;
entity tb is end entity;
architecture sim of tb is
  component enc is port (a : in std_logic := '0'; y : out std_logic); end component;
begin
  u : enc;
end architecture;
",
    )
    .analyze(&["enc.v", "tb.vhd"]);
    let o = t.run(&["tb"]);
    assert_eq!(o.status.code(), Some(2), "{}", err(&o));
    assert!(
        err(&o).contains("E4001") && err(&o).contains("tb.u"),
        "{}",
        err(&o)
    );
}

/// Ports kept apart (`--no-collapse-ports`, as with a debugger or a plugin) give the same run.
#[test]
fn uncollapsed_ports_give_the_same_run() {
    let t = T::new("v-nocollapse");
    t.write("counter.v", COUNTER)
        .write("tb.vhd", COUNTER_TB)
        .analyze(&["counter.v", "tb.vhd"]);
    assert_eq!(t.notes(&["--no-collapse-ports", "tb"]), t.notes(&["tb"]));
}

/// Several drivers on one net (IEEE 1364-2005 4.6): a three-state bus with two enables, a tri0
/// and a tri1 net, a weak driver and a pull on a `bit` port. VHDL sees the resolved `std_ulogic`:
/// `L` and `H` for weak levels, and a `bit` port takes them as '0' and '1'.
#[test]
fn net_with_several_drivers() {
    let t = T::new("v-drivers");
    t.write(
        "tsbus.v",
        "`timescale 1ns/1ps
module tsbus (input ea, input da, input eb, input db,
              output y, output tri0 p, output tri1 q, output w, output tri1 qb);
  assign y = ea ? da : 1'bz;
  assign y = eb ? db : 1'bz;
  assign p = ea ? da : 1'bz;
  assign q = ea ? da : 1'bz;
  assign (weak1, weak0) w = da;
endmodule
",
    )
    .write(
        "tb.vhd",
        "library ieee;
use ieee.std_logic_1164.all;
entity tb is end entity;
architecture sim of tb is
  component tsbus is
    port (ea, da, eb, db : in std_logic; y, p, q, w : out std_logic; qb : out bit);
  end component;
  signal ea, da, eb, db : std_logic := '0';
  signal y, p, q, w : std_logic;
  signal qb : bit;
begin
  u : tsbus port map (ea, da, eb, db, y, p, q, w, qb);
  process
    procedure step(a, d, b, e : std_logic; vy, vp, vq, vw : std_logic) is
    begin
      ea <= a; da <= d; eb <= b; db <= e;
      wait for 1 ns;
      report std_logic'image(y) & std_logic'image(p) & std_logic'image(q) & std_logic'image(w);
      assert y = vy and p = vp and q = vq and w = vw severity failure;
    end procedure;
  begin
    step('0', '0', '0', '0', 'Z', 'L', 'H', 'L');
    step('1', '1', '0', '0', '1', '1', '1', 'H');
    step('0', '0', '1', '0', '0', 'L', 'H', 'L');
    step('1', '1', '1', '0', 'X', '1', '1', 'H');
    assert qb = '1' severity failure;
    wait;
  end process;
end architecture;
",
    )
    .analyze(&["tsbus.v", "tb.vhd"]);
    let n = t.notes(&["tb"]);
    assert_eq!(
        n.iter()
            .map(|l| l.split_once(": ").unwrap().1)
            .collect::<Vec<_>>(),
        [
            "'Z''L''H''L'",
            "'1''1''1''H'",
            "'0''L''H''L'",
            "'X''1''1''H'"
        ]
    );
    let one = out(&t.run(&["tb"]));
    assert_eq!(out(&t.run(&["--threads", "4", "tb"])), one);
}

const DIRECT_TB: &str = "library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
  signal clk, rst : std_logic := '0';
  signal q : std_logic_vector(7 downto 0);
  signal lo : std_logic_vector(3 downto 0);
begin
  dut : entity work.counter generic map (WIDTH => 8) port map (clk => clk, rst => rst, q => q);
  -- A slice, an expression and a constant as actuals, a generic left at the module's default.
  small : entity work.counter port map (clk => clk, rst => not rst, q => lo);
  process
  begin
    rst <= '1'; wait for 5 ns; rst <= '0';
    for i in 1 to 3 loop
      clk <= '1'; wait for 5 ns; clk <= '0'; wait for 5 ns;
      report \"q \" & integer'image(to_integer(unsigned(q))) & \" lo \" & to_string(lo);
    end loop;
    assert unsigned(q) = 3 severity failure;
    wait;
  end process;
end architecture;
";

/// `entity work.counter` instantiates the Verilog module directly: generics and ports come from
/// the module, with the widths its default parameters give; actuals are expressions as for any
/// component, a generic left out keeps the module's default, in the same call as the module's
/// analysis and in a later one, with `-L` or without.
#[test]
fn entity_instantiation_of_a_module() {
    let want = [
        "15000000fs+0: q 1 lo 0000",
        "25000000fs+0: q 2 lo 0000",
        "35000000fs+0: q 3 lo 0000",
    ];
    // One call.
    let t = T::new("v-direct");
    t.write("counter.v", COUNTER)
        .write("tb.vhd", DIRECT_TB)
        .analyze(&["counter.v", "tb.vhd"]);
    assert_eq!(t.notes(&["tb"]), want);
    // Two calls, the module in another library: `entity cells.counter`, no `-L` needed.
    let t = T::new("v-direct-lib");
    t.write("counter.v", COUNTER).write(
        "tb.vhd",
        &DIRECT_TB
            .replace("work.counter", "cells.counter")
            .replace("library ieee;", "library ieee, cells;"),
    );
    let o = t.yavs(&["analyze", "--work=cells:cells", "counter.v"]);
    assert!(o.status.success(), "{}", err(&o));
    let o = t.yavs(&["analyze", "--work=work:lib", "--map=cells:cells", "tb.vhd"]);
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(t.notes(&["--map=cells:cells", "tb"]), want);
}

/// What `entity lib.module` checks, against the module's declaration: the names of generics and
/// ports, an architecture, a module several names match, a header VHDL cannot take, a library
/// that does not have the module, and the case of the name.
#[test]
fn entity_instantiation_is_checked_against_the_module() {
    let tb = |inst: &str| {
        format!(
            "library ieee; use ieee.std_logic_1164.all;
entity tb is end entity;
architecture sim of tb is
  signal clk, rst : std_logic := '0';
  signal q : std_logic_vector(3 downto 0);
begin
  {inst}
end architecture;
"
        )
    };
    let bad = [
        // No such generic or port, a port the module has as an input given an output.
        (
            "u : entity work.counter generic map (DEPTH => 8) port map (clk => clk, rst => rst, q => q);",
            "DEPTH",
        ),
        (
            "u : entity work.counter port map (clk => clk, rst => rst, qq => q);",
            "qq",
        ),
        // A port the module does not have.
        (
            "u : entity work.counter port map (clk => clk, rst => rst, q => open, x => q);",
            "x",
        ),
        // An architecture of a module.
        (
            "u : entity work.counter(rtl) port map (clk => clk, rst => rst, q => q);",
            "E4702",
        ),
        // A module that is not there.
        ("u : entity work.nosuch port map (clk => clk);", "nosuch"),
        // Two modules that differ only in case.
        ("u : entity work.Twin port map (clk => clk);", "E4509"),
        // A header VHDL cannot take.
        ("u : entity work.realport port map (clk => clk);", "E4703"),
    ];
    for (k, (inst, want)) in bad.iter().enumerate() {
        let t = T::new("v-direct-bad");
        t.write("counter.v", COUNTER)
            .write(
                "more.v",
                "`timescale 1ns/1ps
module twin (input clk); endmodule
module TWIN (input clk); endmodule
module realport (input clk, output real r); endmodule
",
            )
            .write("tb.vhd", &tb(inst));
        let o = t.yavs(&[
            "analyze",
            "--work=work:lib",
            "counter.v",
            "more.v",
            "tb.vhd",
        ]);
        assert!(!o.status.success(), "case {k}: analyzed");
        assert!(
            err(&o).contains(want),
            "case {k}: expected {want}:\n{}",
            err(&o)
        );
    }
    // The case of the name: a unique module that differs only in case is the one.
    let t = T::new("v-direct-case");
    t.write("counter.v", COUNTER).write(
        "tb.vhd",
        &tb("u : entity work.COUNTER port map (clk => clk, rst => rst, q => q);"),
    );
    t.analyze(&["counter.v", "tb.vhd"]);
    assert!(t.run(&["tb"]).status.success());
    // A width the generic map changes where the module's range names the parameter, and one
    // where it does not (the range is a number): VHDL reports the mismatch.
    let t = T::new("v-direct-width");
    t.write(
        "m.v",
        "`timescale 1ns/1ps
module fixed (input clk, output [3:0] q); assign q = 4'd5; endmodule
",
    )
    .write(
        "tb.vhd",
        &tb("u : entity work.fixed port map (clk => clk, q => q(2 downto 0));"),
    );
    let o = t.yavs(&["analyze", "--work=work:lib", "m.v", "tb.vhd"]);
    assert!(o.status.success(), "{}", err(&o));
    let o = t.run(&["tb"]);
    assert_eq!(o.status.code(), Some(2), "{}", err(&o));
    assert!(
        err(&o).contains("E2110") || err(&o).contains("E4700"),
        "{}",
        err(&o)
    );
}

/// Port bounds that are constant expressions of the parameters follow the generic map: `$clog2`,
/// shifts, `?:`, a local parameter and a parameter that depends on another one, which keeps
/// following it while the instance leaves it out.
#[test]
fn port_bounds_follow_the_generic_map() {
    let t = T::new("v-bounds");
    t.write(
        "m.v",
        "`timescale 1ns/1ps
module wide #(parameter D = 16, parameter W = 8, parameter B = W * 2)
              (input [$clog2(D)-1:0] a, input [W<<1:0] s, input [(W > 4 ? W : 4)-1:0] c,
               input [B-1:0] d, output [localw-1:0] o);
  localparam localw = W + 1;
  assign o = {1'b0, c};
endmodule
module masked #(parameter W = 8) (input [W & 15:0] a, output y);
  assign y = a[0];
endmodule
",
    )
    .write(
        "tb.vhd",
        "library ieee; use ieee.std_logic_1164.all;
entity tb is end entity;
architecture sim of tb is
  signal a : std_logic_vector(5 downto 0) := (others => '0');
  signal s : std_logic_vector(12 downto 0) := (others => '0');
  signal c : std_logic_vector(5 downto 0) := \"101010\";
  signal d : std_logic_vector(11 downto 0) := (others => '0');
  signal o : std_logic_vector(6 downto 0);
  signal d2 : std_logic_vector(15 downto 0) := (others => '0');
  signal o2 : std_logic_vector(8 downto 0);
  signal s2 : std_logic_vector(16 downto 0) := (others => '0');
  signal a2 : std_logic_vector(3 downto 0) := (others => '0');
  signal c2 : std_logic_vector(7 downto 0) := x\"2A\";
begin
  -- D = 64: a has 6 bits; W = 6: s has 13 bits, c 6, B follows W (12), o has 7.
  u1 : entity work.wide generic map (\\D\\ => 64, W => 6) port map (a => a, s => s, c => c, \\d\\ => d, o => o);
  -- Only B left out of a map that changes W: the default D keeps a at 4 bits, B = W * 2.
  u2 : entity work.wide generic map (W => 8) port map (a => a2, s => s2, c => c2, \\d\\ => d2, o => o2);
  process begin
    wait for 1 ns;
    report to_string(o) & \" \" & to_string(o2);
    wait;
  end process;
end architecture;
",
    )
    .analyze(&["m.v", "tb.vhd"]);
    let n = t.notes(&["tb"]);
    assert_eq!(n.len(), 1, "{n:?}");
    assert!(n[0].ends_with("0101010 000101010"), "{n:?}");
    // A bound VHDL cannot follow: the defaults when the map is empty, an error when it is not.
    let tb = |map: &str| {
        format!(
            "library ieee; use ieee.std_logic_1164.all;
entity tb is end entity;
architecture sim of tb is
  signal a : std_logic_vector(8 downto 0) := (others => '0');
  signal y : std_logic;
begin
  u : entity work.masked {map} port map (a => a, y => y);
end architecture;
"
        )
    };
    t.write("tb2.vhd", &tb(""));
    t.analyze(&["tb2.vhd"]);
    assert!(t.run(&["tb"]).status.success());
    t.write("tb3.vhd", &tb("generic map (W => 4)"));
    let o = t.yavs(&["analyze", "--work=work:lib", "tb3.vhd"]);
    assert!(!o.status.success());
    assert!(
        err(&o).contains("E4706") && err(&o).contains("`a`") && err(&o).contains("&"),
        "{}",
        err(&o)
    );
}

/// `-L`: after the library an `entity lib.module` names, the `-L` libraries are searched; the
/// named library wins, and a miss says where it looked.
#[test]
fn entity_instantiation_searches_the_l_libraries() {
    let tb = "library ieee; use ieee.std_logic_1164.all;
entity tb is end entity;
architecture sim of tb is
  signal clk, rst : std_logic := '0';
  signal q : std_logic_vector(3 downto 0);
begin
  u : entity work.counter port map (clk => clk, rst => rst, q => q);
end architecture;
";
    let t = T::new("v-l");
    t.write("counter.v", COUNTER).write("tb.vhd", tb);
    let o = t.yavs(&["analyze", "--work=cells:cells", "counter.v"]);
    assert!(o.status.success(), "{}", err(&o));
    // Not in `work` and no `-L`: not found.
    let ana = |extra: &[&str]| {
        t.yavs(
            &[
                &["analyze", "--work=work:lib", "--map=cells:cells"],
                extra,
                &["tb.vhd"],
            ]
            .concat(),
        )
    };
    let o = ana(&[]);
    assert!(
        !o.status.success() && err(&o).contains("counter"),
        "{}",
        err(&o)
    );
    // An unmapped -L library, and one that lacks the module, say so.
    let o = ana(&["-L", "nosuch"]);
    assert!(err(&o).contains("E0142"), "{}", err(&o));
    t.write("other.v", "`timescale 1ns/1ps\nmodule other; endmodule\n");
    let o = t.yavs(&["analyze", "--work=empty:empty", "other.v"]);
    assert!(o.status.success(), "{}", err(&o));
    let o = t.yavs(&[
        "analyze",
        "--work=work:lib",
        "--map=cells:cells",
        "--map=empty:empty",
        "-L",
        "empty",
        "tb.vhd",
    ]);
    assert!(!o.status.success());
    assert!(
        err(&o).contains("libraries searched: work, empty"),
        "{}",
        err(&o)
    );
    // With -L cells the module is found there; run and elab take the same option.
    let o = ana(&["-L", "cells"]);
    assert!(o.status.success(), "{}", err(&o));
    let o = t.yavs(&[
        "run",
        "--work=work:lib",
        "--map=cells:cells",
        "-L",
        "cells",
        "tb",
    ]);
    assert!(o.status.success(), "{}", err(&o));
    // A module of the named library wins over the same name in an -L library.
    t.write("work_counter.v", &COUNTER.replace("q + 1", "q + 2"));
    let o = t.yavs(&[
        "analyze",
        "--work=work:lib",
        "--map=cells:cells",
        "-L",
        "cells",
        "work_counter.v",
        "tb.vhd",
    ]);
    assert!(o.status.success(), "{}", err(&o));
}
