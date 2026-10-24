-- A combinational process drives defaults and overrides some of them: the default of an
-- output that an `if` or a `case` arm drives again is not done on that path (opt/drives.rs).
library ieee;
use ieee.std_logic_1164.all;

entity drive_defaults is end;
architecture a of drive_defaults is
  signal sel : integer range 0 to 3 := 0;
  signal c, d, x, y, z : std_logic := '0';
  signal v : std_logic_vector(3 downto 0) := "0000";
begin
  comb : process (c, d, sel)
  begin
    x <= '0';
    y <= '1';
    z <= '0';
    v <= "0000";
    if c = '1' then
      x <= d;
    end if;
    case sel is
      when 0 => y <= '0'; v <= "0101";
      when 1 => y <= d; z <= '1';
      when 2 => v <= "1010";
      when others => null;
    end case;
  end process;

  stim : process
    variable sum : integer := 0;
  begin
    for i in 1 to 60 loop
      sel <= i mod 4;
      c <= '1' when (i / 4) mod 2 = 1 else '0';
      d <= '1' when i mod 3 = 0 else '0';
      wait for 1 ns;
      if x = '1' then sum := sum + 1; end if;
      if y = '1' then sum := sum + 2; end if;
      if z = '1' then sum := sum + 4; end if;
      if v(1) = '1' then sum := sum + 8; end if;
    end loop;
    report "sum=" & integer'image(sum);
    wait;
  end process;
end;
