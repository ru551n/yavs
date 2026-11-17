-- LRM 6.5.7.1, 14.7.3: a formal-side impure conversion reads an `in` port of the enclosing entity that
-- changes in the same cycle as the formal. New or old value of the port?
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity src6 is
  port (o : out unsigned(7 downto 0));
end entity;
architecture a of src6 is
begin
  process begin
    o <= x"12";
    wait for 1 ns; o <= x"34";
    wait for 1 ns; o <= x"56";
    wait;
  end process;
end architecture;
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity wrap6 is
  port (p : in unsigned(7 downto 0); z : out unsigned(7 downto 0));
end entity;
architecture a of wrap6 is
  impure function f(x : unsigned(7 downto 0)) return unsigned is
  begin return x xor p; end function;
begin
  u : entity work.src6 port map (f(o) => z);
end architecture;
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity top is end entity;
architecture a of top is
  signal y, z : unsigned(7 downto 0) := x"01";
begin
  w : entity work.wrap6 port map (p => y, z => z);
  process begin
    wait for 1 ns; y <= x"F0";
    wait for 2 ns; y <= x"0F";
    wait;
  end process;
  process (z) begin report "RES z=" & to_hstring(z); end process;
end architecture;
