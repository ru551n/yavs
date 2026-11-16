-- Aggregates with slice pieces (VHDL-2008) against the normal mode (`W` replaced by a width): a
-- piece with `others`, two pieces and an element, pieces of both directions, a piece of a
-- signal read at run time.
library ieee;
use ieee.std_logic_1164.all;
entity cycle_slicepc is
end entity;
architecture t of cycle_slicepc is
  signal clk, b : std_ulogic := '0';
  signal x : std_ulogic_vector(W - 1 downto 0) := (others => '0');
  signal y : std_ulogic_vector(0 to W - 1) := (others => '0');
  signal p : std_ulogic_vector(W + 7 downto 0);
  signal q : std_ulogic_vector(2 * W downto 0);
  signal u : std_ulogic_vector(0 to W + 3);
  signal r : std_ulogic_vector(W + 7 downto 0);
begin
  p <= (W - 1 downto 0 => x, others => '0');
  q <= (2 * W => b, 2 * W - 1 downto W => x, W - 1 downto 0 => not x);
  u <= (0 to W - 1 => y, others => b);
  reg : process (clk)
  begin
    if rising_edge(clk) then
      r <= (W + 7 downto 8 => x xor p(W - 1 downto 0), 7 downto 0 => b);
    end if;
  end process;
  main : process
    variable s : std_ulogic_vector(15 downto 0) := x"ACE1";
  begin
    for k in 1 to 12 loop
      s := s(14 downto 0) & (s(15) xor s(13) xor s(12) xor s(10));
      x <= x(W - 2 downto 0) & s(0);
      y <= s(1) & y(0 to W - 2);
      b <= s(2);
      wait for 1 ns;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 4 ns;
      report "k=" & integer'image(k) & " p=" & to_hstring(p) & " q=" & to_hstring(q)
        & " u=" & to_string(u) & " r=" & to_hstring(r);
    end loop;
    wait;
  end process;
end architecture;
