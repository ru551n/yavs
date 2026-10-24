-- A process waits for one field of a record whose logic fields share a kernel signal (#686):
-- the report names the field, its value and its drivers, not those of the whole record.
library ieee;
use ieee.std_logic_1164.all;
entity stuck_record is end entity;
architecture a of stuck_record is
  type rec is record
    a, b, c : std_logic;
  end record;
  signal r : rec := ('0', '0', '0');
begin
  w : process
  begin
    wait until r.b = '1';
    report "never";
    wait;
  end process;
  d : process
  begin
    r.a <= '1';
    wait;
  end process;
end architecture;
