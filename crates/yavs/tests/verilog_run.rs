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
        // A net driven twice.
        (
            "module m (input i, output o); assign o = i; assign o = ~i; endmodule",
            "port (i : in std_logic; o : out std_logic);",
            "E4502",
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
    }
}
