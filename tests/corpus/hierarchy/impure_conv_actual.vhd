-- difftest: top=tb_impure_conv_actual std=2008
-- An impure function reading a signal, written as a conversion of a signal in the port map of
-- an `in` port (`b => f(x)`, also on a part): the port follows `x`, and the function reads `y`
-- when it runs (LRM 6.5.7.1). It had been rejected with E2106 at elaboration (cycle mode fuzzer).
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity sub_ica is
  port (b : in unsigned(7 downto 0); c : in unsigned(15 downto 0));
end entity;
architecture a of sub_ica is
begin
  process (b, c)
  begin
    report "b=" & to_hstring(b) & " c=" & to_hstring(c);
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb_impure_conv_actual is
end entity;
architecture a of tb_impure_conv_actual is
  signal x : unsigned(7 downto 0) := x"12";
  signal y : unsigned(7 downto 0) := x"01";
  impure function f(p : unsigned(7 downto 0)) return unsigned is
  begin
    return p xor y;
  end function;
  impure function g(p : unsigned(7 downto 0)) return unsigned is
  begin
    return p + y;
  end function;
begin
  u : entity work.sub_ica
    port map (b => f(x), c(15 downto 8) => x"AB", c(7 downto 0) => g(x));
  process
  begin
    wait for 1 ns;
    x <= x"34";
    wait for 1 ns;
    y <= x"FF";
    wait for 1 ns;
    x <= x"56";
    wait;
  end process;
end architecture;
