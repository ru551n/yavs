-- A process that assigns vectors every activation, mostly the values they hold: the assignments
-- that change nothing are skipped (`Ctx::assign_planes_fast`), but not when an assignment of the
-- same activation to the driver comes first (the last one wins). The monitor hashes every change.
library ieee;
use ieee.std_logic_1164.all;

entity drive_vec is end;
architecture a of drive_vec is
  signal clk : bit := '0';
  signal v : bit_vector(7 downto 0) := x"00";
  signal w : std_logic_vector(15 downto 0) := (others => '0');
  signal z : std_logic_vector(3 downto 0) := "0000";
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;

  p : process
    variable n : integer := 0;
    variable va : bit_vector(7 downto 0) := x"00";
    variable lo, hi : std_logic_vector(7 downto 0) := x"00";
  begin
    wait on clk;
    n := n + 1;
    if n mod 9 = 4 then va := not va; end if;
    if n mod 3 = 0 then lo := x"AA"; else lo := x"00"; end if;
    if n mod 4 = 1 then hi := x"0F"; elsif n mod 4 = 3 then hi := x"00"; end if;
    v <= va;
    -- the first assignment changes the low byte at every third activation, the second restores it
    w(7 downto 0) <= lo;
    w(7 downto 0) <= x"00";
    -- the same value as the signal holds, in two parts of one driver
    w(15 downto 8) <= hi;
    z <= "0000";
    if n >= 120 then done <= true; end if;
  end process;

  mon : process (v, w, z, done)
    variable ev : integer := 0;
    variable h : integer := 0;
  begin
    ev := ev + 1;
    for i in 0 to 7 loop
      if v(i) = '1' then h := (h * 7 + i + 1) mod 65521; end if;
    end loop;
    for i in 0 to 15 loop
      if w(i) = '1' then h := (h * 5 + i + 1) mod 65521; end if;
    end loop;
    if z /= "0000" then h := h + 999; end if;
    if done then
      report "ev=" & integer'image(ev) & " h=" & integer'image(h);
    end if;
  end process;
end;
