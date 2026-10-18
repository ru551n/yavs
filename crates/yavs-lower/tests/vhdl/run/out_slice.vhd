-- difftest: top=out_slice std=2008
library ieee;
use ieee.std_logic_1164.all;
entity out_slice is end;
architecture t of out_slice is
  subtype down is integer range 10 downto 0;
  procedure fill(v : out std_ulogic_vector; x : std_ulogic) is
  begin
    v := (v'range => x);
  end;
  procedure inv(v : inout std_ulogic_vector) is
  begin
    v := not v;
  end;
begin
  process
    variable a : std_ulogic_vector(7 downto 0) := (others => '0');
  begin
    fill(a(5 downto 3), '1');
    inv(a(1 downto 0));
    report to_string(a) & " " & integer'image(down'leftof(5)) & " " & integer'image(down'rightof(5))
      & " " & integer'image(integer'leftof(5));
    wait;
  end process;
end;
