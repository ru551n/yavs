-- Vector gates as concurrent assignments (std_logic_vector of 8 and 70 elements, bit_vector),
-- the inputs changed 40 times: gates the kernel computes itself, with no process code.
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity gate_vec is end;
architecture a of gate_vec is
  signal a, b, x, y, z : std_logic_vector(7 downto 0) := (others => '0');
  signal wa, wb, wx : std_logic_vector(69 downto 0) := (others => '0');
  signal ba, bb, bx : bit_vector(0 to 7);
begin
  x <= a xor b;
  y <= x nand a;
  z <= not y;
  wx <= wa or wb;
  bx <= ba and bb;
  process
    variable s : natural := 0;
  begin
    for k in 1 to 40 loop
      a <= std_logic_vector(to_unsigned(k * 37 mod 256, 8));
      b <= std_logic_vector(to_unsigned(k * 11 mod 256, 8));
      wa(k) <= '1';
      wb(69 - k) <= 'H';
      ba <= to_bitvector(std_logic_vector(to_unsigned(k * 5 mod 256, 8)));
      bb <= to_bitvector(std_logic_vector(to_unsigned(k * 3 mod 256, 8)));
      wait for 1 ns;
      s := s + to_integer(unsigned(z)) + to_integer(unsigned(to_stdlogicvector(bx)));
      if to_x01(wx(k)) = '1' then s := s + 1; end if;
    end loop;
    report "s=" & integer'image(s);
    wait;
  end process;
end;
