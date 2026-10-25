-- A register assigned in pieces every activation, bits and slices in turn and overlapping (the fields of a
-- record before and after a whole assignment): the one-element entries and the vector entries of one
-- run stay direct entries in the order they were made in (`Ctx::assign_code_fast`,
-- `Ctx::assign_planes_fast`, `Ctx::assign_wide_fast`), none goes through the driver's waveform. The monitor
-- hashes every change.
library ieee;
use ieee.std_logic_1164.all;

entity mixed_entries is end;
architecture a of mixed_entries is
  signal clk : bit := '0';
  signal r : std_logic_vector(89 downto 0) := (others => '0');
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;

  p : process
    variable n : integer := 0;
    variable w : std_logic_vector(89 downto 0) := (others => '0');
    variable b : std_logic := '0';
  begin
    wait on clk;
    n := n + 1;
    b := '1' when n mod 3 = 0 else '0';
    if n mod 2 = 0 then w := not w; end if;
    r(5) <= b;
    r(9 downto 6) <= w(3 downto 0);
    r(10) <= not b;
    r(40 downto 12) <= w(28 downto 0);
    r(11) <= b;
    r(89 downto 41) <= w(48 downto 0);
    r(2) <= not b;
    r(1 downto 0) <= w(1 downto 0);
    r(7) <= b;
    if n >= 200 then done <= true; end if;
  end process;

  mon : process (r, done)
    variable ev : integer := 0;
    variable h : integer := 0;
  begin
    ev := ev + 1;
    for i in 0 to 89 loop
      if r(i) = '1' then h := (h * 7 + i + 1) mod 65521; end if;
    end loop;
    if done then
      report "ev=" & integer'image(ev) & " h=" & integer'image(h);
    end if;
  end process;
end;
