-- A clocked process drives inout ports (not direct drivers: the signals have ports) several
-- times an activation, mostly with the value it holds: the repeats of a skipped drive stay on
-- the fast paths of the kernel (`general_assigns`).
library ieee;
use ieee.std_logic_1164.all;

entity drive_repeat_unit is
  port (clk : in std_logic; n : in integer range 0 to 3; h : in std_logic;
        o : inout std_logic; ov : inout std_logic_vector(7 downto 0));
end;
architecture a of drive_repeat_unit is
begin
  process (clk)
  begin
    if rising_edge(clk) then
      o <= h;
      ov <= (others => h);
      for i in 1 to n loop
        o <= h;
        ov <= (others => h);
      end loop;
    end if;
  end process;
end;

library ieee;
use ieee.std_logic_1164.all;

entity drive_repeat is end;
architecture a of drive_repeat is
  signal clk, h, o : std_logic := '0';
  signal n : integer range 0 to 3 := 0;
  signal ov : std_logic_vector(7 downto 0);
begin
  u : entity work.drive_repeat_unit port map (clk, n, h, o, ov);

  stim : process
    variable sum : integer := 0;
  begin
    for i in 1 to 100 loop
      n <= i mod 4;
      h <= '1' when (i / 16) mod 2 = 1 else '0';
      clk <= '1';
      wait for 1 ns;
      clk <= '0';
      wait for 1 ns;
      if o = '1' then sum := sum + i; end if;
      if ov(3) = '1' then sum := sum + 1; end if;
    end loop;
    report "sum=" & integer'image(sum);
    wait;
  end process;
end;
