-- difftest: top=return_select_shared std=2008
-- A small function whose branches share the block that returns a constant, as the code of a short
-- circuit `or` has them, inlined in a loop: every path gets its value.
library ieee;
use ieee.std_logic_1164.all;

entity return_select_shared is
end entity;

architecture a of return_select_shared is
  type int_vec is array (natural range <>) of integer;

  function to_int(v : std_ulogic) return integer is
  begin
    if v = '1' or v = 'H' then
      return 1;
    end if;
    return 0;
  end function;

  function to_ints(v : std_logic_vector) return int_vec is
    variable r : int_vec(v'range);
  begin
    for i in v'range loop
      r(i) := to_int(v(i));
    end loop;
    return r;
  end function;
begin
  process
    variable x : int_vec(0 to 3);
  begin
    x := to_ints(std_logic_vector'("1H0L"));
    report integer'image(x(0)) & integer'image(x(1)) & integer'image(x(2)) & integer'image(x(3));
    report integer'image(to_ints(std_logic_vector'("101"))(0));
    wait;
  end process;
end architecture;
