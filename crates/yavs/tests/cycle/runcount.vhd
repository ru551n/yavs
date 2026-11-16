-- A process outside the region whose result depends on how often it runs (`y <= v` before
-- `v := ...`: the value of its previous run), fed by a register and combinational logic, against
-- the normal mode (`W` replaced by a width): the combinational logic in its fan-in stays in the
-- event kernel, the registers join.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_runcount is
end entity;
architecture t of cycle_runcount is
  signal clk : std_ulogic := '0';
  signal d, q, c, c2, y, s : unsigned(W - 1 downto 0) := (others => '0');
begin
  reg : process (clk)
  begin
    if rising_edge(clk) then
      q <= d;
      s <= y;
    end if;
  end process;
  c <= q + 1;
  c2 <= c + q;
  stale : process (all)
    variable v : unsigned(W - 1 downto 0) := (others => '0');
  begin
    y <= v;
    v := q xor c2;
  end process;
  main : process
    variable r : unsigned(15 downto 0) := x"ACE1";
  begin
    for k in 1 to 12 loop
      r := r(14 downto 0) & (r(15) xor r(13) xor r(12) xor r(10));
      d <= d(W - 2 downto 0) & r(0);
      wait for 1 ns;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 4 ns;
      report "k=" & integer'image(k) & " y=" & to_hstring(y) & " s=" & to_hstring(s);
    end loop;
    wait;
  end process;
end architecture;
