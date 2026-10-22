library ieee; use ieee.std_logic_1164.all;
package pkg_reduce is
  function any_f(d : std_ulogic_vector) return std_ulogic;
  function any_d(d : std_ulogic_vector) return std_ulogic;
  function all_f(d : std_ulogic_vector) return std_ulogic;
end package;
package body pkg_reduce is
  function any_f(d : std_ulogic_vector) return std_ulogic is
    variable v : std_ulogic;
  begin
    v := '0';
    for i in d'range loop
      v := v or d(i);
    end loop;
    return v;
  end function;
  function any_d(d : std_ulogic_vector) return std_ulogic is
    variable v : std_ulogic := '0';
  begin
    for i in d'range loop
      v := v or d(i);
    end loop;
    return v;
  end function;
  -- no gate in the loop: stays a call
  function all_f(d : std_ulogic_vector) return std_ulogic is
    variable v : std_ulogic;
  begin
    v := '0';
    for i in d'range loop
      v := d(i);
    end loop;
    return v;
  end function;
  function wrap(d : std_ulogic_vector) return std_ulogic is
    variable x : std_ulogic := all_f(d);  -- the look-alike is called too
    variable y : std_ulogic := any_f(d);
  begin
    return any_d(d);
  end function;
end package body;
