-- difftest: top=element_results_fault std=2008 exit=1 ignore=fatalmsg
-- A result of `to_unsigned` copied into an element of an array of vectors whose index fails: the
-- index error, in a function called often enough to be compiled. (The index is checked before the
-- operation runs, as in the reference simulators, #1156.)
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity element_results_fault is
end entity;

architecture sim of element_results_fault is
  type a12 is array (natural range <>) of unsigned(11 downto 0);
  function put(v : a12; at, m : integer) return a12 is
    variable r : a12(v'range) := v;
  begin
    for k in r'low to r'high - 1 loop
      r(at + k) := to_unsigned(to_integer(v(k) + v(k + 1)) * m, 12);
    end loop;
    return r;
  end function;
begin
  process is
    variable v : a12(0 to 4);
    variable s : integer := 0;
  begin
    for i in 0 to 2999 loop
      for k in v'range loop
        v(k) := to_unsigned((i * 13 + k) mod 4096, 12);
      end loop;
      v := put(v, 0, 1);
      s := (s + to_integer(v(3))) mod 1000003;
    end loop;
    report "s=" & integer'image(s);
    v := put(v, 2, 1);
    report "not reached";
    wait;
  end process;
end architecture;
