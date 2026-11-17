-- LRM 6.5.7.1, 14.7.3: an impure conversion function of a port reads signals that change in the
-- same cycle as its actual. Does it see their new or their old value?
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity sub is
  port (b : in unsigned(7 downto 0));
end entity;
architecture a of sub is
begin
  process (b) begin report "RES b=" & to_hstring(b); end process;
end architecture;
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity top is end entity;
architecture a of top is
  signal y : unsigned(7 downto 0) := x"01";
  signal x : unsigned(7 downto 0) := x"12";
  signal z : unsigned(7 downto 0) := x"00";
  impure function f(p : unsigned(7 downto 0)) return unsigned is
  begin
    return p xor y xor z;
  end function;
begin
  u : entity work.sub port map (b => f(x));
  process begin
    wait for 1 ns; x <= x"34"; y <= x"F0"; z <= x"0F";
    wait for 1 ns; z <= x"01"; x <= x"00";
    wait;
  end process;
end architecture;
