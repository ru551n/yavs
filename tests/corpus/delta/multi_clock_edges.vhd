-- difftest: top=multi_clock_edges std=2008
-- Processes on edges of several clocks: `process (c1, c2)` made only of edge `if`s and `wait until
-- E1 or E2`, which the kernel resumes only for one of the edges, beside near shapes it must
-- resume at every event (a tail, an `else`, a clock twice, an `on` list without a clock, a level
-- disjunct, two clock ports joined to one signal). The clocks change together (one rising while
-- the other falls), through 'H', 'L' and 'X', one falling while the other holds, and as elements
-- 0 and W-1 of a vector, at widths 1, 63, 64, 65 and 128. Every count and register is reported at
-- the end.
library ieee;
use ieee.std_logic_1164.all;

entity mc_unit is
  generic (W : positive);
  port (c1, c2 : in std_ulogic; vc, d : in std_ulogic_vector(W - 1 downto 0); done : in boolean);
end;

architecture a of mc_unit is
  signal qa, qb, qv, qw : std_ulogic_vector(W - 1 downto 0) := (others => '0');
  signal na, nc, nd, ne, nf, nm, nl, ns, nt : natural := 0;
begin
  pa : process (c1, c2)
  begin
    if rising_edge(c1) then
      qa <= d;
      na <= na + 1;
    end if;
    if falling_edge(c2) then
      qb <= qa xor d;
    end if;
  end process;

  pv : process (vc(0), vc(W - 1))
  begin
    if vc(0)'event and vc(0) = '1' then
      qv <= d;
    end if;
    if falling_edge(vc(W - 1)) then
      qw <= not d;
    end if;
  end process;

  pw : process
  begin
    wait until rising_edge(c1) or (c2'event and c2 = '0');
    nc <= nc + 1;
  end process;

  -- Near shapes: each runs at every event of its clocks.
  pt : process (c1, c2)
  begin
    if rising_edge(c1) then
      nd <= nd + 1;
    end if;
    if rising_edge(c2) then
      nt <= nt + 1;
    end if;
    ne <= ne + 1;
  end process;

  pe : process (c1, c2)
  begin
    if rising_edge(c1) then
      nf <= nf;
    else
      nf <= nf + 1;
    end if;
    if rising_edge(c2) then
      nf <= nf + 2;
    end if;
  end process;

  ps : process (c1, c2)
  begin
    if rising_edge(c1) then
      ns <= ns + 1;
    end if;
    if falling_edge(c1) then
      ns <= ns + 10;
    end if;
    if rising_edge(c2) then
      ns <= ns + 100;
    end if;
  end process;

  pm : process
  begin
    wait on c1 until rising_edge(c1) or rising_edge(c2);
    nm <= nm + 1;
  end process;

  pl : process
  begin
    wait until c1 = '1' or rising_edge(c2);
    nl <= nl + 1;
  end process;

  rep : process (done)
  begin
    if done then
      report "W=" & integer'image(W) & " qa=" & to_hstring(qa) & " qb=" & to_hstring(qb)
        & " qv=" & to_hstring(qv) & " qw=" & to_hstring(qw) & " na=" & integer'image(na)
        & " nc=" & integer'image(nc) & " nd=" & integer'image(nd) & " ne=" & integer'image(ne)
        & " nf=" & integer'image(nf) & " nm=" & integer'image(nm) & " nl=" & integer'image(nl)
        & " ns=" & integer'image(ns) & " nt=" & integer'image(nt);
    end if;
  end process;
end;

library ieee;
use ieee.std_logic_1164.all;

-- Two clock ports that the instance joins to one signal: a rising and a falling edge of it.
entity mc_join is
  port (ca, cb : in std_ulogic; done : in boolean);
end;

architecture a of mc_join is
  signal nr, nf, ns : natural := 0;
begin
  pj : process (ca, cb)
  begin
    if rising_edge(ca) then
      nr <= nr + 1;
    end if;
    if falling_edge(cb) then
      nf <= nf + 1;
    end if;
  end process;

  pk : process (ca, cb)
  begin
    if rising_edge(ca) then
      ns <= ns + 1;
    end if;
    if rising_edge(cb) then
      ns <= ns + 10;
    end if;
  end process;

  rep : process (done)
  begin
    if done then
      report "join nr=" & integer'image(nr) & " nf=" & integer'image(nf) & " ns=" & integer'image(ns);
    end if;
  end process;
end;

library ieee;
use ieee.std_logic_1164.all;

entity multi_clock_edges is end;

architecture t of multi_clock_edges is
  signal c1, c2 : std_ulogic := '0';
  signal vc, d : std_ulogic_vector(127 downto 0) := (others => '0');
  signal done : boolean := false;
  type widths_t is array (0 to 4) of positive;
  constant WIDTHS : widths_t := (1, 63, 64, 65, 128);

  function ul(c : character) return std_ulogic is
  begin
    case c is
      when '1' => return '1';
      when 'H' => return 'H';
      when 'L' => return 'L';
      when 'X' => return 'X';
      when others => return '0';
    end case;
  end function;
begin
  g : for k in WIDTHS'range generate
    u : entity work.mc_unit
      generic map (W => WIDTHS(k))
      port map (c1, c2, vc(WIDTHS(k) - 1 downto 0), d(WIDTHS(k) - 1 downto 0), done);
  end generate;

  j : entity work.mc_join port map (c1, c1, done);

  stim : process
    constant s1 : string := "0101HL01X101001101011111";
    constant s2 : string := "1010L1H0X010110010101010";
  begin
    for i in s1'range loop
      c1 <= ul(s1(i));
      c2 <= ul(s2(i));
      for k in WIDTHS'range loop
        vc(WIDTHS(k) - 1) <= ul(s2((i + k) mod s2'length + 1));
      end loop;
      vc(0) <= ul(s1((i + 3) mod s1'length + 1));
      d(i * 7 mod 128) <= not d(i * 7 mod 128);
      if i mod 3 = 0 then
        d(i * 13 mod 64) <= not d(i * 13 mod 64);
        d(127 - i) <= '1';
      end if;
      wait for 5 ns;
      if i mod 4 = 1 then
        d <= not d;
        wait for 1 ns;
      end if;
    end loop;
    done <= true;
    wait;
  end process;
end;
