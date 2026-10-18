-- yavs-test: std=2019
-- expect: none
-- LRM 2019 4.2.1 (LCS-2016-072b)
library ieee;
use ieee.std_logic_1164.all;
package p is
  function zeros return r of std_logic_vector;
  function iota return r of integer_vector;
end package;
package body p is
  function zeros return r of std_logic_vector is
    variable v : r := (others => '0');
  begin
    return v;
  end function;
  function iota return r of integer_vector is
    variable v : r;
  begin
    for i in v'range loop
      v(i) := i;
    end loop;
    assert r'length = v'length;
    return v;
  end function;
end package body;
