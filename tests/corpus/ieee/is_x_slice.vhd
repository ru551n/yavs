-- difftest: top=is_x_slice std=2008
-- is_x of slices of a logic vector: every element value at every position of a vector of 150
-- elements (three blocks of planes), slices of 1 to 70 elements, ascending and descending vectors,
-- slices that stay inside one block and slices that cross a block edge.
library ieee;
use ieee.std_logic_1164.all;
entity is_x_slice is end;
architecture t of is_x_slice is
  type chars is array (0 to 8) of std_ulogic;
  constant all_values : chars := ('U', 'X', '0', '1', 'Z', 'W', 'L', 'H', '-');
begin
  process
    variable up   : std_ulogic_vector(10 to 159) := (others => '0');
    variable down : std_ulogic_vector(159 downto 10) := (others => '1');
    variable hits, total : integer := 0;
  begin
    for v in chars'range loop
      for p in 10 to 159 loop
        up(p) := all_values(v);
        down(p) := all_values(v);
        for len in 1 to 70 loop
          for off in -2 to 2 loop
            if p + off >= 10 and p + off + len - 1 <= 159 then
              total := total + 1;
              if is_x(up(p + off to p + off + len - 1)) then hits := hits + 1; end if;
              if is_x(down(p + off + len - 1 downto p + off)) then hits := hits + 7; end if;
            end if;
          end loop;
        end loop;
        up(p) := '0';
        down(p) := '1';
      end loop;
    end loop;
    report "total=" & integer'image(total) & " hits=" & integer'image(hits);
    wait;
  end process;
end;
