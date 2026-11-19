-- Cycle mode: a region that waits on clocks and on data. The kernel holds back a wake for an
-- event of a clock without its edge: `ck` (rising, with a reset `arst` from outside), `cf`
-- (falling), `c2` (`'event and = '1'`) and the record field `bs.clk` beside the data `bs.dat`.
-- `cr` has a rising and a falling class and `ckd` is also read as data: they wake it at every
-- event. `din` and `bs.dat` move between edges and in the delta of a falling edge.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_waitfilter is
end entity;
architecture a of cycle_waitfilter is
  type bus_t is record
    clk : std_ulogic;
    dat : std_ulogic_vector(W - 1 downto 0);
  end record;
  signal ck, cf, c2, cr, ckd, arst, ckout : std_ulogic := '0';
  signal bs : bus_t := (clk => '0', dat => (others => '0'));
  signal din, q, qf, q2, qb, qr, qd, qe, y, z : std_ulogic_vector(W - 1 downto 0) := (others => '0');
  signal cnt : unsigned(W - 1 downto 0) := (others => '0');
  signal done : boolean := false;
begin
  -- The region.
  p1 : process (ck, arst)
  begin
    if arst = '1' then
      q <= (others => '0');
      cnt <= (others => '0');
    elsif rising_edge(ck) then
      q <= din;
      cnt <= cnt + 1;
    end if;
  end process;

  p2 : process (cf)
  begin
    if falling_edge(cf) then
      qf <= din xor q;
    end if;
  end process;

  p3 : process (c2)
  begin
    if c2'event and c2 = '1' then
      q2 <= bs.dat;
    end if;
  end process;

  p4 : process (bs.clk)
  begin
    if rising_edge(bs.clk) then
      qb <= std_ulogic_vector(cnt) xor din;
    end if;
  end process;

  p5 : process (cr)
  begin
    if rising_edge(cr) then
      qr <= q;
    end if;
  end process;

  p6 : process (cr)
  begin
    if falling_edge(cr) then
      qd <= qr xor qf;
    end if;
  end process;

  p7 : process (ckd)
  begin
    if rising_edge(ckd) then
      qe <= q2 xor qb;
    end if;
  end process;

  ckout <= ckd;
  y <= din xor q;
  z <= bs.dat xor qb;

  -- Outside: the clocks (half period, start).
  g_ck : process
  begin
    wait for 0 ns;
    while not done loop
      ck <= '1';
      wait for 5 ns;
      ck <= '0';
      wait for 5 ns;
    end loop;
    wait;
  end process;

  g_cf : process
  begin
    wait for 2 ns;
    while not done loop
      cf <= '1';
      wait for 7 ns;
      cf <= '0';
      wait for 7 ns;
    end loop;
    wait;
  end process;

  g_c2 : process
  begin
    wait for 1 ns;
    while not done loop
      c2 <= '1';
      wait for 3 ns;
      c2 <= '0';
      wait for 3 ns;
    end loop;
    wait;
  end process;

  g_cr : process
  begin
    wait for 0 ns;
    while not done loop
      cr <= '1';
      wait for 6 ns;
      cr <= '0';
      wait for 6 ns;
    end loop;
    wait;
  end process;

  g_ckd : process
  begin
    wait for 4 ns;
    while not done loop
      ckd <= '1';
      wait for 9 ns;
      ckd <= '0';
      wait for 9 ns;
    end loop;
    wait;
  end process;

  -- Outside: `bs`, its clock and data together at times; `din` between edges and with them.
  stim : process
    variable k : natural := 0;
  begin
    for i in 1 to 60 loop
      bs.clk <= '1';
      if i mod 3 = 0 then
        din(i mod W) <= not din(i mod W);
      end if;
      wait for 4 ns;
      bs.clk <= '0';
      if i mod 4 = 1 then
        bs.dat(i * 7 mod W) <= not bs.dat(i * 7 mod W);
      end if;
      wait for 1 ns;
      if i mod 5 = 2 then
        din(W - 1) <= not din(W - 1);
      end if;
      wait for 3 ns;
      if i = 20 then
        arst <= '1', '0' after 2 ns;
      end if;
    end loop;
    done <= true;
    wait;
  end process;

  monitor : process (ck, cf, c2, cr, ckd, bs.clk, din, bs.dat)
  begin
    if not done then
      report "t=" & time'image(now) & " q=" & to_hstring(q) & " qf=" & to_hstring(qf)
        & " q2=" & to_hstring(q2) & " qb=" & to_hstring(qb) & " qr=" & to_hstring(qr)
        & " qd=" & to_hstring(qd) & " qe=" & to_hstring(qe) & " y=" & to_hstring(y)
        & " z=" & to_hstring(z) & " ckout=" & to_string(ckout) & " cnt=" & to_hstring(cnt);
    end if;
  end process;
end architecture;
