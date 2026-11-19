-- Instances sharing one compiled body: u0 and u1 (K = 3) share one, u2 and u3 (K = 5, a constant
-- folded into the code) another; each drives its own signals from its own input. With FAIL,
-- u1's input leaves the range of `t` at 75 ns: the error names u1, as in the normal mode.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity cycle_share_unit is
  generic (K : natural := 3; N : natural := 80);
  port (clk : in std_logic; d : in unsigned(15 downto 0); q : out unsigned(15 downto 0));
end entity;

architecture a of cycle_share_unit is
  signal acc : unsigned(15 downto 0) := (others => '0');
  signal t : natural range 0 to 60000 := 0;
begin
  p : process (clk)
    variable v : unsigned(15 downto 0);
  begin
    if rising_edge(clk) then
      v := acc xor d;
      for i in 0 to N - 1 loop
        v := (v(14 downto 0) & v(15)) + to_unsigned(K * i + 1, 16);
      end loop;
      acc <= v;
      t <= to_integer(d);
    end if;
  end process;
  q <= acc;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity cycle_share is
  generic (FAIL : boolean := false);
end entity;

architecture a of cycle_share is
  signal clk : std_logic := '0';
  signal d0, d1, d2, d3, q0, q1, q2, q3 : unsigned(15 downto 0) := (others => '0');
begin
  u0 : entity work.cycle_share_unit generic map (K => 3) port map (clk, d0, q0);
  u1 : entity work.cycle_share_unit generic map (K => 3) port map (clk, d1, q1);
  u2 : entity work.cycle_share_unit generic map (K => 5) port map (clk, d2, q2);
  u3 : entity work.cycle_share_unit generic map (K => 5) port map (clk, d3, q3);

  main : process
  begin
    for c in 1 to 10 loop
      d0 <= to_unsigned(c * 7, 16);
      d1 <= to_unsigned(c * 11, 16);
      if FAIL and c = 7 then
        d1 <= to_unsigned(65000, 16);
      end if;
      d2 <= to_unsigned(c * 13, 16);
      d3 <= to_unsigned(c * 17, 16);
      wait for 5 ns;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      report "c=" & integer'image(c) & " q0=" & to_hstring(q0) & " q1=" & to_hstring(q1)
        & " q2=" & to_hstring(q2) & " q3=" & to_hstring(q3);
    end loop;
    wait;
  end process;
end architecture;
