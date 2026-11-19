-- Cycle mode: the settle at an edge runs only the steps the clocked members of the classes that
-- had one, and the inputs read, reach. Clocks ca (8 ns), cc (4 ns, its rising edges with ca's)
-- and cb (14 ns) from one process, cg gated from ca in the kernel: logic of one clock alone, logic
-- two clocks share, a register that keeps its value, a memory written on ca and read into cb's
-- logic, a reset made by a register of cb for registers of ca, and a synchroniser reset from
-- outside. A monitor prints every register, and the memories (one written on cb alone), half a
-- nanosecond after each step of the clocks; reports of ca and cc in one delta come in process order.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_edgecone is
  generic (N : natural := 120);
end entity;
architecture a of cycle_edgecone is
  subtype word is unsigned(W-1 downto 0);
  signal ca, cb, cg, ce : std_ulogic := '0';
  signal cc : std_ulogic := '1';
  signal arst : std_ulogic := '1';
  signal ra, ya, za, rb, yb, zb, x, sa, sb, k, kk, kq, q, yq, qz, gq, g2, gz : word := (others => '0');
  signal rd, rq, h, h2 : word := (others => '0');
  type mem_t is array (0 to 4096) of word;
  signal mem, mem2 : mem_t := (others => (others => '0'));
  signal wa, ri, wb2 : natural range 0 to 4096 := 0;
  signal rs1, rs2, rs3 : std_ulogic := '0';
  -- One word with an input field and a field a register of cb drives.
  type pair_t is record
    a : unsigned(3 downto 0);
    b : unsigned(3 downto 0);
  end record;
  signal two : pair_t := (x"0", x"0");
  signal twa : unsigned(3 downto 0) := x"0";
  signal done : boolean := false;
begin
  clk : process
  begin
    for t in 1 to N loop
      wait for 1 ns;
      if t mod 4 = 0 then
        ca <= not ca;
      end if;
      if t mod 2 = 0 then
        cc <= not cc;
      end if;
      if t mod 7 = 0 then
        cb <= not cb;
      end if;
    end loop;
    done <= true;
    wait;
  end process;
  cg <= ca and ce;
  -- Logic of ca alone.
  pa : process (ca)
  begin
    if rising_edge(ca) then
      ra <= ra + 1;
      za <= ya;
    end if;
  end process;
  ya <= ra(0) & ra(W-1 downto 1) when ra(0) = '1' else not ra;
  -- Logic of cb alone.
  pb : process (cb)
  begin
    if rising_edge(cb) then
      rb <= rb + 1;
      zb <= yb;
    end if;
  end process;
  yb <= rb xor shift_left(rb, 1);
  -- Logic both share, sampled on each.
  x <= ya + yb;
  psa : process (ca)
  begin
    if rising_edge(ca) then
      sa <= x;
    end if;
  end process;
  psb : process (cb)
  begin
    if rising_edge(cb) then
      sb <= x;
    end if;
  end process;
  -- A register that keeps its value most edges, its logic sampled on cb.
  pk : process (ca)
  begin
    if rising_edge(ca) then
      k <= shift_right(ra, 3);
    end if;
  end process;
  kk <= k + 1;
  pkq : process (cb)
  begin
    if rising_edge(cb) then
      kq <= kk;
    end if;
  end process;
  -- cc's rising edges come with ca's, in the same delta; logic of cc's own.
  pq : process (cc)
  begin
    if rising_edge(cc) then
      q <= q + ra;
      qz <= yq;
    end if;
  end process;
  yq <= q xor shift_right(q, 1);
  -- The gated clock's own logic.
  pg : process (cg)
  begin
    if rising_edge(cg) then
      gq <= gq + 1;
      gz <= g2;
    end if;
  end process;
  g2 <= gq xor shift_left(gq, 2);
  -- A memory written on ca, read by logic into a register of cb.
  pw : process (ca)
  begin
    if rising_edge(ca) then
      mem(wa) <= not ra;
      wa <= (wa + 1) mod 4097;
    end if;
  end process;
  rd <= mem(ri);
  -- A memory written on cb alone, read outside.
  pw2 : process (cb)
  begin
    if rising_edge(cb) then
      mem2(wb2) <= rb;
      wb2 <= (wb2 + 1) mod 4097;
    end if;
  end process;
  ppb : process (cb)
  begin
    if rising_edge(cb) then
      two.b <= two.b + 1;
    end if;
  end process;
  ppa : process (ca)
  begin
    if rising_edge(ca) then
      twa <= two.a;
    end if;
  end process;
  pr : process (cb)
  begin
    if rising_edge(cb) then
      rq <= rd;
      ri <= (ri + 1) mod 4097;
    end if;
  end process;
  -- A synchroniser on cb with a reset from outside, and a reset cb makes for ca's registers.
  psy : process (cb, arst)
  begin
    if arst = '1' then
      rs1 <= '1';
      rs2 <= '1';
    elsif rising_edge(cb) then
      rs1 <= '0';
      rs2 <= rs1;
      rs3 <= rb(0);
    end if;
  end process;
  ph : process (ca, rs2)
  begin
    if rs2 = '1' then
      h <= (others => '0');
    elsif rising_edge(ca) then
      h <= h + 1;
    end if;
  end process;
  ph2 : process (ca, rs3)
  begin
    if rs3 = '1' then
      h2 <= (others => '0');
    elsif rising_edge(ca) then
      h2 <= h2 + ra;
    end if;
  end process;
  -- Reports of two classes in one delta, in process order.
  pna : process (ca)
  begin
    if rising_edge(ca) then
      if ra < 3 then
        report "na=" & std_ulogic'image(ra(0));
      end if;
    end if;
  end process;
  pnc : process (cc)
  begin
    if rising_edge(cc) then
      if ra < 3 then
        report "nc=" & std_ulogic'image(q(0));
      end if;
    end if;
  end process;
  pnd : process (ca)
  begin
    if rising_edge(ca) then
      if ra < 3 then
        report "nd=" & std_ulogic'image(h(0));
      end if;
    end if;
  end process;
  main : process
  begin
    wait for 3 ns;
    arst <= '0';
    two.a <= x"5";
    wait for 10 ns;
    ce <= '1';
    wait for 30 ns;
    ce <= '0';
    wait for 20.5 ns;
    two.a <= x"a";
    arst <= '1';
    wait for 1 ns;
    arst <= '0';
    wait for 12 ns;
    ce <= '1';
    wait;
  end process;
  mon : process
  begin
    wait for 500 ps;
    while not done loop
      report "t=" & time'image(now) & " a=" & to_hstring(ra) & " " & to_hstring(za) & " "
        & to_hstring(sa) & " b=" & to_hstring(rb) & " " & to_hstring(zb) & " " & to_hstring(sb)
        & " k=" & to_hstring(kq) & " q=" & to_hstring(q) & " " & to_hstring(qz) & " g=" & to_hstring(gz) & " "
        & to_hstring(gq) & " m=" & to_hstring(rq) & " h=" & to_hstring(h) & " " & to_hstring(h2)
        & " r=" & std_ulogic'image(rs2) & std_ulogic'image(rs3) & " p=" & to_hstring(two.b) & " "
        & to_hstring(twa) & " w=" & to_hstring(mem(ri))
        & " " & to_hstring(mem2((wb2 + 4096) mod 4097));
      wait for 1 ns;
    end loop;
    wait;
  end process;
end architecture;
