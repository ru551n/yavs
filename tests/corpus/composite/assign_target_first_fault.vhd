-- difftest: top=assign_target_first_fault std=2008 exit=1 ignore=fatalmsg
-- The index of an element target is checked before the value is made (#1156): the value's
-- truncation warning is reported for the three elements in range only, then the index error.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity assign_target_first_fault is
end entity;

architecture sim of assign_target_first_fault is
  type a12 is array (natural range <>) of unsigned(11 downto 0);
begin
  process is
    variable r : a12(0 to 4);
  begin
    for k in 0 to 3 loop
      r(2 + k) := to_unsigned(5000 + k, 12);
    end loop;
    report "not reached";
    wait;
  end process;
end architecture;
