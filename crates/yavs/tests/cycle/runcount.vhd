-- Processes outside the region whose result depends on the deltas their inputs change in, fed
-- by a register and combinational logic, against the normal mode (`W` replaced by a width):
-- `stale` keeps a value of its previous run (`y <= v` before `v := ...`); in the design below the
-- top, `list` reads `c3`, which its sensitivity list does not name, and `mask` keeps a value but
-- is rejected for its `'Z'` first. The logic in their fan-in stays in the event kernel, the
-- registers join, and so does `e3`, read only by a concurrent assignment rejected for its `'Z'`.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_runcount_dut is
  port (
    q : in unsigned(W - 1 downto 0);
    y2, y3, t : out unsigned(W - 1 downto 0);
    zz : out std_logic
  );
end entity;
architecture rtl of cycle_runcount_dut is
  signal c3, e4, e3 : unsigned(W - 1 downto 0) := (others => '0');
begin
  c3 <= not q;
  list : process (q)
  begin
    y2 <= q + c3;
  end process;
  e4 <= rotate_left(q, 1);
  mask : process (all)
    variable v : unsigned(W - 1 downto 0) := (others => '0');
  begin
    y3 <= v;
    v := q xor e4;
    zz <= 'Z';
  end process;
  e3 <= rotate_right(q, 1);
  t <= e3 when q(0) = '1' else (others => 'Z');
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_runcount is
end entity;
architecture t of cycle_runcount is
  signal clk : std_ulogic := '0';
  signal zz : std_logic := '0';
  signal d, q, c, c2, y, s : unsigned(W - 1 downto 0) := (others => '0');
  signal y2, s2, y3, s3, t : unsigned(W - 1 downto 0) := (others => '0');
begin
  reg : process (clk)
  begin
    if rising_edge(clk) then
      q <= d;
      s <= y;
      s2 <= y2;
      s3 <= y3;
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
  dut : entity work.cycle_runcount_dut port map (q => q, y2 => y2, y3 => y3, t => t, zz => zz);
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
      report "k=" & integer'image(k) & " y=" & to_hstring(y) & " s=" & to_hstring(s) &
        " s2=" & to_hstring(s2) & " s3=" & to_hstring(s3) & " t=" & to_hstring(t);
    end loop;
    wait;
  end process;
end architecture;
