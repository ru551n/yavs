-- A chain of 1000 inverters (concurrent assignments), the clock rippling through for 2000 cycles:
-- a long delta-cycle chain, one event per stage per edge.
-- Head-to-head design (tools/bench/AGENTS.md); written for yavs.
library ieee; use ieee.std_logic_1164.all;
entity bench_rtl_chain is generic (N : natural := 1000; CYC : natural := 2000); end;
architecture a of bench_rtl_chain is
  signal clk : std_logic := '0';
  signal c : std_logic_vector(0 to N);
  signal done : boolean := false;
  signal o : std_logic;
begin
  clk <= not clk after 5 ns when not done;
  c(0) <= clk;
  g: for i in 1 to N generate
    c(i) <= not c(i-1);
  end generate;
  o <= c(N);
  process
    variable n : natural := 0;
  begin
    for k in 1 to CYC loop wait on o; n := n + 1; end loop;
    report "n=" & integer'image(n) & " last=" & std_logic'image(o);
    done <= true; wait;
  end process;
end;
