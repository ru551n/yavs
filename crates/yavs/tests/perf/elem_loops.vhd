-- Loops that copy, reverse and fill the elements of logic vectors: each runs all its iterations
-- but the last at once (element loops), so the operations executed do not grow with the length.
library ieee;
use ieee.std_logic_1164.all;
entity elem_loops is end;
architecture a of elem_loops is
begin
  process
    variable a : std_logic_vector(63 downto 0) := x"0123456789ABCDEF";
    variable b : std_logic_vector(63 downto 0) := (others => '0');
    variable c : std_logic_vector(0 to 63) := (others => '0');
    variable acc : integer := 0;
  begin
    for rep in 1 to 20 loop
      for i in a'range loop b(i) := a(i); end loop;
      for i in b'range loop c(63 - i) := b(i); end loop;
      for i in a'range loop a(i) := c(63 - i); end loop;
      for i in b'range loop b(i) := '1'; end loop;
      a(rep) := not a(rep);
      for i in a'range loop
        if a(i) = '1' then acc := (acc + i * rep) mod 100003; end if;
      end loop;
    end loop;
    report "acc=" & integer'image(acc);
    wait;
  end process;
end;
