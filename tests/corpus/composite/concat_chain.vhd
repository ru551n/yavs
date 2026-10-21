-- difftest: top=concat_chain std=2008
-- Chains of `&` (LRM 9.2.5): operands of any mix of arrays and elements, null operands and the
-- bounds of the result.
library ieee;
use ieee.std_logic_1164.all;
entity concat_chain is end;
architecture t of concat_chain is
  function ident(n : integer) return integer is
  begin
    return n;
  end;
  function info(s : string) return string is
  begin
    return "[" & s & "] " & integer'image(s'left) & " " & integer'image(s'right) & " "
      & integer'image(s'length);
  end;
  function info(v : std_logic_vector) return string is
  begin
    return integer'image(v'left) & " " & integer'image(v'right) & " " & integer'image(v'length);
  end;
begin
  process
    variable e1 : string(ident(5) to ident(4));
    variable e2 : string(ident(3) to ident(2));
    variable e3 : string(ident(10) to ident(9));
    variable a : string(7 to 9) := "xyz";
    variable b : string(ident(20) to ident(21)) := "pq";
    variable l1 : std_logic_vector(7 downto 4) := "10XZ";
    variable l2 : std_logic_vector(ident(3) downto ident(3)) := "1";
    variable l3 : std_logic_vector(ident(1) downto ident(2));
  begin
    report info(a & b & a);
    report info('<' & a & b & '>');
    report info(e1 & a & e2 & b & e3);
    report info(a & e1 & 'q' & e2);
    report info('a' & 'b' & 'c' & 'd');
    report info(a & ('m' & b) & a);
    report info(l1 & l2 & l3);
    report info(l3 & l1 & '1' & l2);
    report info('0' & l1 & '1' & '1' & l1);
    wait;
  end process;
end;
