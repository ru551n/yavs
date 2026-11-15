-- Cycle mode: several clocks. c1 and c2 (twice the rate) come from one process in one delta, as
-- the clock model of a double-pumped datapath makes them; c3 has a period of its own, from
-- another process (its edges meet c1's and c2's now and then); cu starts at 'U' (an edge for
-- `'event and = '1'`, none for `rising_edge`); cg is c1 gated by a register of c1, made in the
-- kernel, so its edges come one delta after c1's. Registers of each clock read registers of the
-- others at coincident edges (also registers no process of their own clock reads), the two
-- edges of c1 read each other, and logic between clocks is sampled on c2 (one delta: in the
-- region) and on c3 and cg (maybe another delta: the logic stays in the kernel).
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_clocks is
  generic (N : natural := 24);
end entity;
architecture a of cycle_clocks is
  signal c1 : std_ulogic := '0';
  signal c2 : std_ulogic := '1';
  signal c3 : std_ulogic := '0';
  signal cu : std_ulogic;
  signal cg, en : std_ulogic := '0';
  signal done : boolean := false;
  signal a1, a2, a3, af, s, t, u : unsigned(7 downto 0) := x"00";
  signal x1, x2, x3, xf, x4, x5, w2, g1, gx, gs, nl, nr, y0 : unsigned(7 downto 0) := x"00";
begin
  -- c2, ahead of c1's process: a register only c1 reads.
  p0 : process (c2)
  begin
    if rising_edge(c2) then
      y0 <= a2 + 1;
    end if;
  end process;
  -- c1: reads c2's register at coincident edges and the falling-edge register.
  p1 : process (c1)
  begin
    if rising_edge(c1) then
      a1 <= a1 + 3;
      x1 <= a2 xor y0;
      xf <= af;
      en <= a1(1) or a1(2);
    end if;
  end process;
  -- c2: reads c1's registers at coincident edges (the values before the edge); nothing on c1
  -- reads x1.
  p2 : process (c2)
  begin
    if rising_edge(c2) then
      a2 <= a2 + 5;
      x2 <= a1 xor x1;
    end if;
  end process;
  -- The falling edge of c1 reads the rising edge's register.
  pf : process (c1)
  begin
    if falling_edge(c1) then
      af <= af + a1 + 1;
    end if;
  end process;
  -- Logic of both pump clocks sampled on c2: edges in one delta.
  s <= a1 + a2;
  p3 : process (c2)
  begin
    if rising_edge(c2) then
      x3 <= s;
    end if;
  end process;
  pw : process
  begin
    wait until rising_edge(c2);
    w2 <= w2 + a1;
  end process;
  -- c3: registers of c1 and c2 read directly, and logic of them.
  t <= a1 xor a3;
  p4 : process (c3)
  begin
    if rising_edge(c3) then
      a3 <= a3 + 1;
      x4 <= a1 + a2;
      x5 <= t;
    end if;
  end process;
  -- c1 gated in the kernel: c1's register read directly, and through logic.
  cg <= c1 and en;
  u <= a1 + 1;
  pg : process (cg)
  begin
    if rising_edge(cg) then
      g1 <= g1 + 1;
      gx <= a1;
    end if;
  end process;
  pgs : process (cg)
  begin
    if rising_edge(cg) then
      gs <= u;
    end if;
  end process;
  -- A clock from 'U': the two edge tests differ at its first edge.
  pl : process (cu)
  begin
    if cu'event and cu = '1' then
      nl <= nl + 1;
    end if;
  end process;
  pr : process (cu)
  begin
    if rising_edge(cu) then
      nr <= nr + 1;
    end if;
  end process;
  clocks : process
  begin
    for k in 0 to 4 * N - 1 loop
      wait for 2.5 ns;
      c2 <= not c2;
      if k mod 2 = 1 then
        c1 <= not c1;
      end if;
    end loop;
    done <= true;
    wait;
  end process;
  clock3 : process
  begin
    while not done loop
      wait for 3 ns;
      c3 <= not c3;
    end loop;
    wait;
  end process;
  clocku : process
  begin
    wait for 13 ns;
    while not done loop
      cu <= '1';
      wait for 4 ns;
      cu <= '0';
      wait for 4 ns;
    end loop;
    wait;
  end process;
  mon : process
  begin
    wait for 1.2 ns;
    while not done loop
      wait for 10 ns;
      report "a1=" & to_hstring(a1) & " a2=" & to_hstring(a2) & " a3=" & to_hstring(a3)
        & " af=" & to_hstring(af) & " x1=" & to_hstring(x1) & " x2=" & to_hstring(x2)
        & " x3=" & to_hstring(x3) & " xf=" & to_hstring(xf) & " x4=" & to_hstring(x4)
        & " x5=" & to_hstring(x5) & " w2=" & to_hstring(w2) & " g1=" & to_hstring(g1)
        & " gx=" & to_hstring(gx) & " gs=" & to_hstring(gs) & " nl=" & to_hstring(nl)
        & " nr=" & to_hstring(nr);
    end loop;
    wait;
  end process;
end architecture;
