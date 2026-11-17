-- difftest: top=assert_edge std=2008 ignore=order
-- (ignore=order: assertions that fire in one cycle report in an order the LRM leaves open.)
-- Concurrent assertions `assert not (<clock edge> and ...)`: nothing is reported in a cycle
-- without that edge, so their processes may resume only on it. Covered: assertions that fire, the
-- edge first or later in the `and` chain, a falling edge, metavalue operands, operand events in
-- the edge's cycle and between edges, and forms that must fire without an edge (an `or`, a
-- negated edge, no `not`, an `and` with the edge that is not negated).
library ieee;
use ieee.std_logic_1164.all;

entity assert_edge is end;

architecture t of assert_edge is
  signal clk : std_logic := '0';
  signal valid, ctx : std_logic := '0';
  signal size : natural := 0;
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  valid <= '1' after 12 ns, '0' after 17 ns, '1' after 25 ns, 'X' after 35 ns, '1' after 43 ns,
           'U' after 51 ns, '1' after 62 ns, '0' after 75 ns;
  ctx <= '1' after 30 ns, '0' after 40 ns, 'H' after 60 ns, '0' after 70 ns;
  size <= 2 after 20 ns, 4 after 33 ns, 1 after 48 ns, 5 after 66 ns;
  done <= true after 90 ns;

  a1 : assert not (rising_edge(clk) and valid = '1' and ctx = '0')
    report "a1 beat without a context at " & time'image(now) severity note;

  a2 : assert not (rising_edge(clk)
                   and valid = '1'
                   and (ctx = '1' or size > 3))
    report "a2 at " & time'image(now) & " size " & integer'image(size) severity note;

  a3 : assert not (valid /= '0' and falling_edge(clk))
    report "a3 falling at " & time'image(now) & " valid " & std_ulogic'image(valid) severity note;

  a4 : assert not (rising_edge(clk) and valid /= '0' and valid /= '1')
    report "a4 metavalue at " & time'image(now) severity note;

  -- Not an edge conjunct: these fire in cycles without an edge.
  n1 : assert not (rising_edge(clk) or valid = '1')
    report "n1 at " & time'image(now) & " clk " & std_ulogic'image(clk) severity note;

  n2 : assert not (not rising_edge(clk) and ctx = '1')
    report "n2 at " & time'image(now) & " clk " & std_ulogic'image(clk) severity note;

  n3 : assert rising_edge(clk) or size < 4
    report "n3 at " & time'image(now) & " size " & integer'image(size) severity note;

  n4 : assert size < 4 and rising_edge(clk)
    report "n4 at " & time'image(now) & " clk " & std_ulogic'image(clk) severity note;
end architecture;
