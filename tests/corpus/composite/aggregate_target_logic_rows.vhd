-- difftest: top=tb_aggregate_target_logic_rows std=2008 ref=hand
-- Aggregate targets of arrays of logic arrays: elements and 2008 pieces, signals and variables.
-- ref=hand: the first reference simulator stops with an aggregate length error on the concurrent
-- assignment (its pieces count the scalars); the expected text is the second one's.
library ieee; use ieee.std_logic_1164.all;
entity tb_aggregate_target_logic_rows is end entity;
architecture a of tb_aggregate_target_logic_rows is
  type vv is array (natural range <>) of std_logic_vector;
  signal d : vv(2 downto 0)(3 downto 0);
  signal o, e1, e2 : std_logic_vector(3 downto 0);
  signal i : std_logic_vector(3 downto 0) := x"1";
  signal clk : std_logic := '0';
begin
  (o, d) <= d & i when rising_edge(clk);
  process
    variable x, y : std_logic_vector(3 downto 0);
    variable z : vv(0 to 1)(3 downto 0);
    variable w : vv(0 to 3)(3 downto 0);
  begin
    w := (x"A", x"B", x"C", x"D");
    (x, y, z) := w;
    report to_string(x) & " " & to_string(y) & " " & to_string(z(0)) & " " & to_string(z(1));
    (z, x, y) := w;
    report to_string(x) & " " & to_string(y) & " " & to_string(z(0)) & " " & to_string(z(1));
    (e1, e2) <= vv'(x"3", x"4"), vv'(x"5", x"6") after 2 ns;
    wait for 1 ns;
    report to_string(e1) & to_string(e2);
    wait for 2 ns;
    report to_string(e1) & to_string(e2);
    for k in 1 to 5 loop
      i <= x"0";
      i(k mod 4) <= '1';
      clk <= '1';
      wait for 1 ns;
      clk <= '0';
      wait for 1 ns;
      report to_string(o) & " " & to_string(d(2)) & " " & to_string(d(1)) & " " & to_string(d(0));
    end loop;
    wait;
  end process;
end architecture;
