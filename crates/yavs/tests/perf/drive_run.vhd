-- A process that assigns its outputs one after the other, mostly the values they hold: the
-- drives are skipped as a run (pre.rs `drive_runs`) while nothing changes, and the ones that
-- change, even in the middle of the run, are made. The monitor counts the events it sees.
library ieee;
use ieee.std_logic_1164.all;

entity drive_run is end;
architecture a of drive_run is
  signal clk : bit := '0';
  signal a, b, c, d : bit := '0';
  signal l1, l2 : std_logic := '0';
  signal v : bit_vector(3 downto 0) := "0000";
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;

  p : process
    variable va, vb, vc, vd : bit := '0';
    variable w1, w2 : std_logic := '0';
    variable n : integer := 0;
  begin
    wait on clk;
    n := n + 1;
    if n mod 7 = 3 then va := not va; end if;
    if n mod 11 = 5 then vc := not vc; end if;
    if n mod 13 = 6 then w2 := not w2; end if;
    if n mod 17 = 1 then vd := '1'; elsif n mod 17 = 9 then vd := '0'; end if;
    a <= va;
    b <= vb;
    c <= vc;
    d <= vd;
    l1 <= w1;
    l2 <= w2;
    v(0) <= va;
    v(1) <= vb;
    v(2) <= vc;
    if n mod 5 = 0 then
      v(1) <= '1';
    elsif n mod 5 = 1 then
      v(1) <= '0';
    end if;
    if n >= 200 then done <= true; end if;
  end process;

  mon : process (a, b, c, d, l1, l2, v, done)
    variable ev : integer := 0;
    variable h : integer := 0;
  begin
    ev := ev + 1;
    h := (h * 3 + ev + boolean'pos(done)) mod 65521;
    if a = '1' then h := h + 1; end if;
    if c = '1' then h := h + 2; end if;
    if d = '1' then h := h + 4; end if;
    if l2 = '1' then h := h + 8; end if;
    if v(1) = '1' then h := h + 16; end if;
    if done then
      report "ev=" & integer'image(ev) & " h=" & integer'image(h);
    end if;
  end process;
end;
