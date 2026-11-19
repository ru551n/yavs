-- Signed division of the most negative value by -1 (`W` replaced by a width): `rem` and `mod`
-- give 0 and `numeric_std`'s `/` wraps to the most negative value in the normal mode; the region
-- stopped with an overflow error there. Also a `rem` and a `mod` by a one-element -1, and an
-- integer type of 64 bits.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
package cycle_divmin_pkg is
  type long is range -9223372036854775807 - 1 to 9223372036854775807;
end package;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.cycle_divmin_pkg.all;
entity cycle_divmin_u is
  port (clk : in std_ulogic; a, b : in signed(W - 1 downto 0); m1 : in signed(0 downto 0);
        x, y : in long;
        qr, qm, qd : out signed(W - 1 downto 0); r1, m1o : out signed(0 downto 0); xr, xm : out long);
end entity;
architecture a of cycle_divmin_u is
begin
  reg : process (clk)
  begin
    if rising_edge(clk) then
      qr <= a rem b;
      qm <= a mod b;
      qd <= a / b;
      r1 <= a rem m1;
      m1o <= a mod m1;
      xr <= x rem y;
      xm <= x mod y;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.cycle_divmin_pkg.all;
entity cycle_divmin is
end entity;
architecture t of cycle_divmin is
  constant MN : signed(W - 1 downto 0) := (W - 1 => '1', others => '0');
  constant MX : signed(W - 1 downto 0) := (W - 1 => '0', others => '1');
  constant ONE : signed(W - 1 downto 0) := (0 => '1', others => '0');
  type pairs is array (natural range <>) of integer;
  signal clk : std_ulogic := '0';
  signal a, b : signed(W - 1 downto 0) := (others => '0');
  signal m1 : signed(0 downto 0) := "1";
  signal x, y : long := 1;
  signal qr, qm, qd : signed(W - 1 downto 0);
  signal r1, m1o : signed(0 downto 0);
  signal xr, xm : long;
begin
  u : entity work.cycle_divmin_u port map (clk, a, b, m1, x, y, qr, qm, qd, r1, m1o, xr, xm);
  main : process
  begin
    for k in 0 to 7 loop
      case k is
        when 0 => a <= MN; b <= (others => '1'); x <= long'low; y <= -1;
        when 1 => a <= MN; b <= ONE; x <= long'low; y <= 1;
        when 2 => a <= MX; b <= (others => '1'); x <= long'high; y <= -1;
        when 3 => a <= MN; b <= MN; x <= long'low; y <= long'low;
        when 4 => a <= (others => '1'); b <= (others => '1'); x <= -7; y <= 2;
        when 5 => a <= MX; b <= MN; x <= 7; y <= -2;
        when 6 => a <= MN or ONE; b <= (others => '1'); x <= long'low + 1; y <= -1;
        when others => a <= (others => '0'); b <= (others => '1'); x <= 0; y <= -1;
      end case;
      wait for 1 ns;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 4 ns;
      report "k=" & integer'image(k) & " " & to_hstring(qr) & " " & to_hstring(qm) & " " & to_hstring(qd)
        & " " & to_hstring(r1) & " " & to_hstring(m1o) & " " & long'image(xr) & " " & long'image(xm);
    end loop;
    wait;
  end process;
end architecture;
