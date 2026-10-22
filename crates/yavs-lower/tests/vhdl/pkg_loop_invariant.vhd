library ieee;
use ieee.std_logic_1164.all;

package pkg_loop_invariant is
  type mem_t is array (natural range <>) of std_logic_vector;
  type mat_t is array (natural range <>, natural range <>) of std_logic;
end package;

package body pkg_loop_invariant is
  function to_mat(m : mem_t) return mat_t is
    variable r : mat_t(m'range, m(m'left)'range);
  begin
    for i in m'range loop
      for j in m(i)'range loop
        r(i, j) := m(i)(j);
      end loop;
    end loop;
    return r;
  end function;
end package body;
