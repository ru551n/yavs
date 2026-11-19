-- Cycle mode: edge runs that would change nothing (the idle rule). Two clocks `ca` and `cb` with
-- edges at other times; `cnt` counts on `ca` while `en` holds, `sat` counts on `ca` up to its top
-- and then keeps it (idle with no input moving), `q` loads `d xor cnt` on `cb` (a register of the
-- other clock read directly), `tg` toggles on `cb` while `d(0)` holds (one class busy while the
-- other is idle), `r` has an asynchronous reset `arst` from outside and stops at 3.
-- The stimulus holds everything for stretches, then moves `en` in the delta of an edge, `d`
-- between edges and pulses `arst`; a monitor prints after each edge of either clock.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_idle is
end entity;
architecture a of cycle_idle is
  signal ca, cb, en, arst, tg : std_ulogic := '0';
  signal d, q, y : std_ulogic_vector(W - 1 downto 0) := (others => '0');
  signal cnt, sat : unsigned(W - 1 downto 0) := (others => '0');
  signal r : unsigned(1 downto 0) := "00";
  signal done : boolean := false;
begin
  -- The region.
  pa : process (ca)
  begin
    if rising_edge(ca) then
      if en = '1' then
        cnt <= cnt + 1;
      end if;
      if sat < 5 then
        sat <= sat + 1;
      end if;
    end if;
  end process;

  pb : process (cb)
  begin
    if rising_edge(cb) then
      q <= d xor std_ulogic_vector(cnt);
      if d(0) = '1' then
        tg <= not tg;
      end if;
    end if;
  end process;

  pr : process (ca, arst)
  begin
    if arst = '1' then
      r <= "00";
    elsif rising_edge(ca) then
      if r /= 3 then
        r <= r + 1;
      end if;
    end if;
  end process;

  y <= q xor std_ulogic_vector(sat);

  -- Outside: the clocks; `en` moves in the delta of an edge of `ca`.
  mka : process
  begin
    for i in 1 to 120 loop
      ca <= '1';
      if i = 30 or i = 36 or i = 80 or i = 81 then
        en <= not en;
      end if;
      wait for 5 ns;
      ca <= '0';
      wait for 5 ns;
    end loop;
    done <= true;
    wait;
  end process;

  mkb : process
  begin
    wait for 3 ns;
    while not done loop
      cb <= '1';
      wait for 7 ns;
      cb <= '0';
      wait for 7 ns;
    end loop;
    wait;
  end process;

  -- Outside: `d` between edges, a pulse of `arst`.
  stim : process
  begin
    wait for 402 ns;
    d <= (others => '1');
    wait for 300 ns;
    arst <= '1';
    wait for 2 ns;
    arst <= '0';
    wait for 200 ns;
    d(0) <= '0';
    wait;
  end process;

  monitor : process (ca, cb)
  begin
    if not done and (ca'event or cb'event) then
      report "t=" & time'image(now) & " cnt=" & to_hstring(cnt) & " sat=" & to_hstring(sat)
        & " q=" & to_hstring(q) & " tg=" & to_string(tg) & " r=" & to_string(r) & " y=" & to_hstring(y);
    end if;
  end process;
end architecture;
