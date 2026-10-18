-- difftest: top=port_conv_composite std=2008
-- Conversion functions and type conversions of array ports in a port map (#74).
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity port_conv_composite_sub is port (i : in integer; o : out integer := 0; v : in std_logic_vector(3 downto 0); w : out unsigned(3 downto 0)); end;
architecture a of port_conv_composite_sub is begin
  o <= i + 1;
  w <= unsigned(v) + 1;
end;
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
package port_conv_composite_conv is
  function int2slv(i : integer) return std_logic_vector;
end;
package body port_conv_composite_conv is
  function int2slv(i : integer) return std_logic_vector is
  begin return std_logic_vector(to_unsigned(i, 8)); end;
end;
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all; use work.port_conv_composite_conv.all;
entity port_conv_composite is end;
architecture t of port_conv_composite is
  signal a : unsigned(3 downto 0) := "0011";
  signal b : std_logic_vector(3 downto 0);
  signal x : std_logic_vector(7 downto 0);
  signal n : natural;
begin
  u : entity work.port_conv_composite_sub port map (
    i => to_integer(a),
    int2slv(o) => x,
    v => std_logic_vector(a),
    std_logic_vector(w) => b);
  process begin
    for k in 1 to 3 loop
      a <= a + 2; wait for 1 ns;
      report "x=" & to_string(x) & " b=" & to_string(b);
    end loop;
    wait;
  end process;
end;
