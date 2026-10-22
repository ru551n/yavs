-- difftest: top=static_descriptors std=2008
-- Arrays of locally static bounds in instance data: constants and signals indexed by enumeration
-- and subtype marks, their attributes, a loop over 'range of an instance constant, a port whose
-- actual has other bounds.
library ieee;
use ieee.std_logic_1164.all;

entity sub is
  port (x : in std_logic_vector(3 downto 0); y : out std_logic_vector(0 to 3));
end entity;
architecture a of sub is
begin
  y(0) <= x(3);
  y(1) <= x(2);
  y(2) <= x(1);
  y(3) <= x(0);
end architecture;

library ieee;
use ieee.std_logic_1164.all;
entity static_descriptors is end entity;

architecture test of static_descriptors is
  type color_t is (red, green, blue, white);
  subtype warm_t is color_t range green to blue;
  type table_t is array (color_t, boolean) of integer;
  type warm_vec_t is array (warm_t) of integer;
  constant tab : table_t := ((1, 2), (3, 4), (5, 6), (7, 8));
  constant warm : warm_vec_t := (10, 20);
  constant taps : integer_vector(2 to 6) := (7, 0, 9, 0, 11);
  signal sig : table_t := ((0, 0), (0, 0), (0, 0), (0, 0));
  signal vec : std_logic_vector(10 downto 4) := "0000000";
  signal bus_a : std_logic_vector(15 downto 12) := "1010";
  signal bus_b : std_logic_vector(0 to 3);
begin
  u : entity work.sub port map (x => bus_a, y => bus_b);

  process
    variable sum : integer := 0;
  begin
    for c in color_t loop
      for b in boolean loop
        sum := sum + tab(c, b) * (color_t'pos(c) + 1);
        sig(c, b) <= tab(c, b) + integer(boolean'pos(b));
      end loop;
    end loop;
    report "sum=" & integer'image(sum);
    for i in taps'range loop
      if taps(i) /= 0 then sum := sum + taps(i) * i; end if;
    end loop;
    report "sum=" & integer'image(sum) & " warm=" & integer'image(warm(green) + warm(blue));
    report "attrs " & integer'image(tab'length(1)) & integer'image(tab'length(2)) & integer'image(taps'left)
      & integer'image(taps'right) & boolean'image(taps'ascending) & integer'image(warm'length);
    wait for 1 ns;
    vec(6 downto 5) <= "11";
    for i in vec'range loop
      sum := sum + i;
    end loop;
    wait for 1 ns;
    report "sig " & integer'image(sig(blue, true)) & " vec " & to_string(vec) & " b " & to_string(bus_b)
      & " " & integer'image(bus_b'left) & integer'image(vec'length);
    wait;
  end process;
end architecture;
