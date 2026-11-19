-- Concurrent assignments with a delay below the top, outside the region, fed by logic that
-- glitches for one delta in the normal mode (`W` replaced by a width): an inertial delay
-- (`y1`) and a reject limit (`y3`) remove a pending transaction on the glitch, a transport delay
-- that differs by alternative (`y2`) keeps the glitch's transaction. Their logic stays in the
-- event kernel; `y4`, without a delay, is a function of its last run and its logic joins.
library ieee;
use ieee.std_logic_1164.all;
entity cycle_inertial_dut is
  port (
    clk : in std_ulogic;
    y1, y3, y4 : out std_ulogic_vector(W - 1 downto 0) := (others => '0');
    y2 : out std_ulogic := '0'
  );
end entity;
architecture rtl of cycle_inertial_dut is
  signal q, b1, c1, b2, b3, c3, b4, c4 : std_ulogic_vector(W - 1 downto 0) := (others => '0');
  signal g2 : std_ulogic := '0';
begin
  reg : process (clk)
  begin
    if rising_edge(clk) then
      q <= not q;
    end if;
  end process;
  b1 <= q;
  c1 <= q xnor b1;
  y1 <= c1 after 3 ns;
  b2 <= not q;
  g2 <= q(0) xnor b2(0);
  y2 <= transport '1' after 1 ns when g2 = '1' else '0' after 2 ns;
  b3 <= q;
  c3 <= q xnor b3;
  y3 <= reject 2 ns inertial c3 after 3 ns;
  b4 <= q;
  c4 <= q xnor b4;
  y4 <= c4 or q;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
entity cycle_inertial is
end entity;
architecture t of cycle_inertial is
  signal clk, y2 : std_ulogic := '0';
  signal y1, y3, y4 : std_ulogic_vector(W - 1 downto 0) := (others => '0');
begin
  dut : entity work.cycle_inertial_dut port map (clk => clk, y1 => y1, y2 => y2, y3 => y3, y4 => y4);
  main : process
  begin
    for k in 1 to 22 loop
      if k <= 16 then
        clk <= not clk;
      end if;
      wait for 500 ps;
      report "k=" & integer'image(k) & " y1=" & to_hstring(y1) & " y2=" & std_ulogic'image(y2) &
        " y3=" & to_hstring(y3) & " y4=" & to_hstring(y4);
      wait for 500 ps;
    end loop;
    wait;
  end process;
end architecture;
