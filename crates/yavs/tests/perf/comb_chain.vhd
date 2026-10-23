-- 24 inverters as concurrent assignments, the clock rippling through for 40 cycles: a long
-- delta chain, a process wake and an event per stage and edge.
library ieee; use ieee.std_logic_1164.all;
entity comb_chain is generic (N : natural := 24; CYC : natural := 40); end;
architecture a of comb_chain is
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
