-- Fills and copies of vectors of thousands of elements, at every alignment: whole blocks, one
-- shared bit offset, different offsets, and overlapping shifts within one vector. Every element is
-- checked against its expected value; the report counts mismatches.
library ieee;
use ieee.std_logic_1164.all;
entity wide_vec is end;
architecture a of wide_vec is
  function pat(i : natural) return std_logic is
  begin
    if (i * 7 + i / 5) mod 3 = 0 then return '1';
    elsif i mod 11 = 4 then return 'X';
    else return '0';
    end if;
  end function;
begin
  process
    variable a, b, c : std_logic_vector(0 to 4999);
    variable bad : natural := 0;
    variable one : std_logic := '1';
    variable zero : std_logic := '0';
  begin
    for rep in 1 to 3 loop
      for i in a'range loop a(i) := pat(i); end loop;
      -- whole blocks, aligned
      b := (others => 'U');
      b(0 to 4095) := a(0 to 4095);
      for i in b'range loop
        if i < 4096 then if b(i) /= pat(i) then bad := bad + 1; end if;
        elsif b(i) /= 'U' then bad := bad + 1; end if;
      end loop;
      -- the same offset inside the blocks
      b := (others => 'Z');
      b(70 to 4500) := a(70 to 4500);
      for i in b'range loop
        if i >= 70 and i <= 4500 then if b(i) /= pat(i) then bad := bad + 1; end if;
        elsif b(i) /= 'Z' then bad := bad + 1; end if;
      end loop;
      -- different offsets
      b := (others => 'W');
      b(5 to 4200) := a(9 to 4204);
      for i in b'range loop
        if i >= 5 and i <= 4200 then if b(i) /= pat(i + 4) then bad := bad + 1; end if;
        elsif b(i) /= 'W' then bad := bad + 1; end if;
      end loop;
      -- an overlapping shift to the right and to the left within one vector
      c := a;
      c(64 to 4999) := c(0 to 4935);
      for i in c'range loop
        if i >= 64 then if c(i) /= pat(i - 64) then bad := bad + 1; end if;
        elsif c(i) /= pat(i) then bad := bad + 1; end if;
      end loop;
      c := a;
      c(0 to 4935) := c(64 to 4999);
      for i in c'range loop
        if i <= 4935 then if c(i) /= pat(i + 64) then bad := bad + 1; end if;
        elsif c(i) /= pat(i) then bad := bad + 1; end if;
      end loop;
      c := a;
      c(100 to 4999) := c(36 to 4935);
      for i in c'range loop
        if i >= 100 then if c(i) /= pat(i - 64) then bad := bad + 1; end if;
        elsif c(i) /= pat(i) then bad := bad + 1; end if;
      end loop;
      -- fills
      b := (others => one);
      b(3 to 4000) := (others => zero);
      for i in b'range loop
        if i >= 3 and i <= 4000 then if b(i) /= '0' then bad := bad + 1; end if;
        elsif b(i) /= '1' then bad := bad + 1; end if;
      end loop;
    end loop;
    report "wide=" & integer'image(bad);
    wait;
  end process;
end;
