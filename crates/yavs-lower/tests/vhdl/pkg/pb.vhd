library ieee;
use ieee.std_logic_1164.all;
package pb is
  impure function new_id return natural;
  impure function get_c1 return natural;
  impure function get_v return std_ulogic_vector;
  signal s : std_ulogic_vector(0 to 3) := "0101";
end package;
package body pb is
  type counter_t is protected
    impure function next_id return natural;
  end protected;
  type counter_t is protected body
    variable n : natural := 0;
    impure function next_id return natural is
    begin
      n := n + 1;
      return n;
    end;
  end protected body;
  shared variable cnt : counter_t;
  impure function new_id return natural is
  begin
    return cnt.next_id;
  end;
  constant c1 : natural := new_id;
  constant v : std_ulogic_vector(1 to 3) := (1 => '1', others => std_ulogic'val(c1 + 1));
  impure function get_c1 return natural is
  begin
    return c1;
  end;
  impure function get_v return std_ulogic_vector is
  begin
    return v;
  end;
end package body;
