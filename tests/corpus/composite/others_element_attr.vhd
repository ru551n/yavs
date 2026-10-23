-- difftest: top=others_element_attr std=2008
-- An others aggregate assigned to a variable of x'element, where x is a formal array whose
-- element bounds are open: the aggregate takes the element bounds of the actual.
library ieee;
use ieee.std_logic_1164.all;

package pkg is
  type slv_vector is array (natural range <>) of std_logic_vector;
  function f (x : slv_vector) return natural;
end package;

package body pkg is
  function f (x : slv_vector) return natural is
    variable c : x'element;
  begin
    c := (others => '1');
    assert c'left = x(x'left)'left and c'right = x(x'left)'right report "bounds" severity failure;
    return c'length;
  end function;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use work.pkg.all;

entity others_element_attr is end;
architecture behav of others_element_attr is
  constant cv : slv_vector (1 to 2) (5 downto 2) := (x"5", x"a");
  constant dv : slv_vector (1 to 2) (0 to 6) := ("0000000", "1111111");
begin
  process begin
    assert f (cv) = 4 severity failure;
    assert f (dv) = 7 severity failure;
    report "ok";
    wait;
  end process;
end;
