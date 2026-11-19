-- `numeric_std` shifts and rotations by a count chosen at run time, of any width (`W` replaced
-- by a width): a combinational process as the elementwise unit's byte rotation and a register,
-- against the normal mode; counts from 0 past twice the width.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_wideshift_u is
  port (clk : in std_ulogic; x : in std_ulogic_vector(W - 1 downto 0); n : in natural;
        rr, rl : out std_ulogic_vector(W - 1 downto 0);
        sl, sr, sa : out std_ulogic_vector(W - 1 downto 0));
end entity;
architecture a of cycle_wideshift_u is
begin
  rot : process (all)
    variable flat : unsigned(W - 1 downto 0);
  begin
    flat := unsigned(x);
    rr <= std_ulogic_vector(rotate_right(flat, n));
    rl <= std_ulogic_vector(rotate_left(flat, n));
  end process;
  reg : process (clk)
  begin
    if rising_edge(clk) then
      sl <= std_ulogic_vector(shift_left(unsigned(x), n));
      sr <= std_ulogic_vector(shift_right(unsigned(x), n));
      sa <= std_ulogic_vector(shift_right(signed(x), n));
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_wideshift is
end entity;
architecture t of cycle_wideshift is
  signal clk : std_ulogic := '0';
  signal x : std_ulogic_vector(W - 1 downto 0) := (others => '0');
  signal n : natural := 0;
  signal rr, rl, sl, sr, sa : std_ulogic_vector(W - 1 downto 0);
begin
  u : entity work.cycle_wideshift_u port map (clk, x, n, rr, rl, sl, sr, sa);
  main : process
    variable s : unsigned(31 downto 0) := x"ACE1F00D";
  begin
    for k in 0 to 2 * W + 3 loop
      for b in x'range loop
        s := s(30 downto 0) & (s(31) xor s(21) xor s(1) xor s(0));
        x(b) <= s(0);
      end loop;
      n <= k;
      if k = 2 * W + 3 then
        n <= 1000;
      end if;
      wait for 1 ns;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 4 ns;
      report "k=" & integer'image(k) & " " & to_hstring(rr) & " " & to_hstring(rl) & " "
        & to_hstring(sl) & " " & to_hstring(sr) & " " & to_hstring(sa);
    end loop;
    wait;
  end process;
end architecture;
