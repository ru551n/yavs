-- Cycle mode: clocked bodies skipped while idle, and the settle only where a body changed a word.
-- On `ca`: `sat` counts to 5 and keeps it; `cnt` counts while `en` holds and `w2` copies it (a
-- register read by its writer and another body: next-state words); `q` loads `din xor dd`; `rr`
-- counts to 3 with an asynchronous reset `arst` from outside; `x` counts to 3, cleared by `rq`, a
-- register of `cb`; `acc` is a clocked variable adding `din` while `go` holds (`seen`, one starting
-- at 'U', runs the second image of the exact contract until then), `c2` counts in the same edges
-- and only `r2` reads it (two bodies that change in one edge); `v(idx) <= din(0)` writes a
-- vector at a run-time index, `mv(idx) := din(0)` a variable at one while `go` holds (`vq` sees
-- it an edge later, with no input moved in between);
-- `mem` is a memory written at `wa` while `we` holds. On `cb`: `rq` loads `trig`, `z` loads
-- `q xor sat` (registers of the other clock), `mq` reads `mem` at `ra`.
-- The settle: `y` from `q` and `x`, `mo` from `mem`.
-- The stimulus holds everything for stretches, then moves each input once or twice: between
-- edges, in the delta of an edge of `ca` (`en`, `dd`), `arst` and `trig` as pulses between two
-- edges of `ca`.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_blockskip is
end entity;
architecture a of cycle_blockskip is
  type mem_t is array (0 to 15) of std_ulogic_vector(W - 1 downto 0);
  signal ca, cb, en, dd, go, arst, trig, rq, we : std_ulogic := '0';
  signal din, q, w2, z, y, mo, mq, accq : std_ulogic_vector(W - 1 downto 0) := (others => '0');
  signal v, vq : std_ulogic_vector(W - 1 downto 0) := (others => '0');
  signal cnt, sat, c2, r2 : unsigned(W - 1 downto 0) := (others => '0');
  signal rr, x : unsigned(1 downto 0) := "00";
  signal wa, ra : natural range 0 to 15 := 0;
  signal idx : natural range 0 to 255 := 0;
  signal mem : mem_t := (others => (others => '0'));
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
      q <= din xor (W - 1 downto 0 => dd);
    end if;
  end process;

  pw : process (ca)
  begin
    if rising_edge(ca) then
      w2 <= std_ulogic_vector(cnt);
    end if;
  end process;

  pr : process (ca, arst)
  begin
    if arst = '1' then
      rr <= "00";
    elsif rising_edge(ca) then
      if rr /= 3 then
        rr <= rr + 1;
      end if;
    end if;
  end process;

  px : process (ca)
  begin
    if rising_edge(ca) then
      if rq = '1' then
        x <= "00";
      elsif x /= 3 then
        x <= x + 1;
      end if;
    end if;
  end process;

  pv : process (ca)
    variable acc : unsigned(W - 1 downto 0) := (others => '0');
    variable seen : std_ulogic_vector(W - 1 downto 0);
  begin
    if rising_edge(ca) then
      if go = '1' then
        acc := acc + unsigned(din);
        seen := din;
      end if;
      accq <= std_ulogic_vector(acc) xor (seen and (W - 1 downto 0 => '0'));
      if (idx mod W) = idx then
        v(idx) <= din(0);
      end if;
    end if;
  end process;

  pc : process (ca)
  begin
    if rising_edge(ca) then
      if go = '1' then
        c2 <= c2 + 1;
      end if;
    end if;
  end process;

  pd : process (ca)
  begin
    if rising_edge(ca) then
      r2 <= c2;
    end if;
  end process;

  pz : process (ca)
    variable mv : std_ulogic_vector(W - 1 downto 0) := (others => '0');
  begin
    if rising_edge(ca) then
      vq <= mv;
      if go = '1' and (idx mod W) = idx then
        mv(idx) := din(0);
      end if;
    end if;
  end process;

  pm : process (ca)
  begin
    if rising_edge(ca) then
      if we = '1' then
        mem(wa) <= din;
      end if;
    end if;
  end process;

  pb : process (cb)
  begin
    if rising_edge(cb) then
      rq <= trig;
      z <= q xor std_ulogic_vector(sat);
      mq <= mem(ra);
    end if;
  end process;

  y <= q xor std_ulogic_vector(resize(x, W));
  mo <= mem(ra) xor w2;

  -- Outside: the clocks, `cb` at a third of the rate of `ca` with edges at the same times (one
  -- delta class); `en` and `dd` move in the delta of an edge of `ca`.
  mka : process
  begin
    for i in 1 to 160 loop
      ca <= '1';
      if i mod 3 = 1 then
        cb <= '1';
      end if;
      if i = 30 or i = 36 or i = 100 or i = 101 then
        en <= not en;
      end if;
      if i = 120 then
        dd <= '1';
      end if;
      wait for 5 ns;
      ca <= '0';
      if i mod 3 = 2 then
        cb <= '0';
      end if;
      wait for 5 ns;
    end loop;
    done <= true;
    wait;
  end process;

  -- Outside: the inputs between edges, pulses of `arst` and `trig` between two edges of `ca`.
  stim : process
  begin
    wait for 402 ns;
    din <= (others => '1');
    wait for 100 ns;
    go <= '1';
    wait for 30 ns;
    go <= '0';
    wait for 100 ns;
    arst <= '1';
    wait for 2 ns;
    arst <= '0';
    wait for 100 ns;
    idx <= W - 1;
    din(0) <= '0';
    wait for 50 ns;
    wa <= 3;
    we <= '1';
    wait for 10 ns;
    we <= '0';
    wait for 40 ns;
    ra <= 3;
    wait for 60 ns;
    trig <= '1';
    wait for 2 ns;
    trig <= '0';
    wait for 100 ns;
    din(0) <= '1';
    wait;
  end process;

  monitor : process (ca, cb)
  begin
    if not done and (ca'event or cb'event) then
      report "t=" & time'image(now) & " cnt=" & to_hstring(cnt) & " sat=" & to_hstring(sat)
        & " q=" & to_hstring(q) & " w2=" & to_hstring(w2) & " rr=" & to_string(rr)
        & " x=" & to_string(x) & " acc=" & to_hstring(accq) & " v=" & to_hstring(v)
        & " vq=" & to_hstring(vq) & " r2=" & to_hstring(r2) & " z=" & to_hstring(z) & " y=" & to_hstring(y) & " mo=" & to_hstring(mo)
        & " mq=" & to_hstring(mq);
    end if;
  end process;
end architecture;
