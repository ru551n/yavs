-- The actual of an `in` port the kernel updates (a type conversion in the port map, which is not
-- collapsed) driven by members, against the normal mode (`W` replaced by a width): a register and
-- a combinational process drive it, the instance's register and a combinational process read the
-- formal, and the testbench reads both sides.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_portin_u is
  port (clk : in std_ulogic; a, b : in signed(W - 1 downto 0); q : out signed(W - 1 downto 0);
        s : out signed(W - 1 downto 0));
end entity;
architecture a of cycle_portin_u is
begin
  reg : process (clk)
  begin
    if rising_edge(clk) then
      q <= a - b;
    end if;
  end process;
  s <= a xor b;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_portin is
end entity;
architecture t of cycle_portin is
  signal clk : std_ulogic := '0';
  signal d, r, c : std_ulogic_vector(W - 1 downto 0) := (others => '0');
  signal q, s : signed(W - 1 downto 0);
begin
  drive : process (clk)
  begin
    if rising_edge(clk) then
      r <= d;
    end if;
  end process;
  c <= not r;
  u : entity work.cycle_portin_u
    port map (clk => clk, a => signed(r), b => signed(c), q => q, s => s);
  main : process
    variable x : std_ulogic_vector(15 downto 0) := x"ACE1";
  begin
    for k in 1 to 12 loop
      x := x(14 downto 0) & (x(15) xor x(13) xor x(12) xor x(10));
      d <= d(W - 2 downto 0) & x(0);
      wait for 1 ns;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 4 ns;
      report "k=" & integer'image(k) & " r=" & to_hstring(r) & " c=" & to_hstring(c)
        & " q=" & to_hstring(q) & " s=" & to_hstring(s);
    end loop;
    wait;
  end process;
end architecture;
