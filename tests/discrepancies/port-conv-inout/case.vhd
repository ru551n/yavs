-- LRM 6.5.7.1, 14.7.7.3: both conversions of an inout port. The formal b follows the actual
-- when only the second driver of the resolved actual changes it (3 ns and 5 ns).
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity sub is port (b : inout integer); end entity;
architecture a of sub is
begin
  process begin
    b <= 5;
    wait for 2 ns;
    b <= b + 4;
    wait;
  end process;
  process begin
    wait on b;
    report "RES b " & integer'image(b) & " at " & integer'image(now / 1 ns);
  end process;
end architecture;
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity top is end entity;
architecture a of top is
  function to_slv(i : integer) return std_logic_vector is
  begin
    return std_logic_vector(to_unsigned(i mod 16, 4));
  end function;
  function to_int(v : std_logic_vector) return integer is
  begin
    return to_integer(unsigned(to_01(v)));
  end function;
  signal t : std_logic_vector(3 downto 0) := "ZZZZ";
begin
  u : entity work.sub port map (to_slv(b) => to_int(t));
  t <= "ZZZZ", "ZZ1Z" after 3 ns, "ZZZZ" after 5 ns;
  process begin
    wait for 7 ns;
    report "RES t " & to_string(t);
    wait;
  end process;
end architecture;
