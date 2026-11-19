-- Cycle mode: loop groups, processes reading through one another (or themselves) what they
-- drive, with no loop element by element. Two processes through the fields of one record; three
-- needing three passes; one reading a signal it drives after assigning it; a group with the reset
-- branch of a clocked process; a register of the groups' values; a reader outside the region.
-- With FAIL, a check in a group fails on settled values.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_loops is
  generic (FAIL : boolean := false);
end entity;
architecture a of cycle_loops is
  type pair_t is record
    a, b : unsigned(7 downto 0);
  end record;
  type table_t is array (0 to 7) of unsigned(7 downto 0);
  constant TBL : table_t := (x"11", x"22", x"33", x"44", x"55", x"66", x"77", x"88");
  signal clk : std_ulogic := '0';
  signal rst : std_ulogic := '1';
  signal din, din2 : unsigned(7 downto 0) := x"00";
  signal r : pair_t := (x"00", x"00");
  signal o1 : unsigned(7 downto 0);
  signal x1, x2, x3, y1, y2, z1 : unsigned(7 downto 0);
  signal s, t : unsigned(7 downto 0);
  signal w1, w2, q : unsigned(7 downto 0);
  signal k1, k2 : integer range 0 to 7 := 0;
  signal fo : unsigned(7 downto 0);
  signal rq, ob : unsigned(7 downto 0) := x"00";
  signal done : boolean := false;
begin
  -- Through the fields of one record: din -> r.a -> r.b -> o1.
  pa : process (all)
  begin
    r.a <= din + 1;
    o1 <= r.b + 3;
  end process;
  pb : process (all)
  begin
    r.b <= r.a xor din2;
  end process;
  -- din -> x1 -> y1 -> z1 -> x2 -> y2 -> x3: two steps back in the order ca, cb, cc.
  ca : process (all)
  begin
    x1 <= din + 1;
    x2 <= z1 + 1;
    x3 <= y2 xor din2;
  end process;
  cb : process (all)
  begin
    y1 <= x1 + 2;
    y2 <= x2 + 4;
  end process;
  cc : process (all)
  begin
    z1 <= y1 + 8;
  end process;
  -- Reads `s` after assigning it: the value from before the process ran.
  sr : process (all)
  begin
    s <= x"00";
    t <= s;
    s <= din xor x"5A";
  end process;
  -- A reset branch in a group: while rst holds q follows w1.
  pw : process (all)
  begin
    w1 <= din + 7;
    w2 <= q + 1;
  end process;
  pq : process (clk, rst, w1)
  begin
    if rst = '1' then
      q <= w1;
    elsif rising_edge(clk) then
      q <= q + 2;
    end if;
  end process;
  -- A check of a group on settled values (k2 = 8 when k1 = 7).
  pf : process (all)
  begin
    if FAIL then
      k1 <= to_integer(din(2 downto 0));
    else
      k1 <= to_integer(din(1 downto 0));
    end if;
    fo <= TBL(k2);
  end process;
  pg : process (all)
  begin
    k2 <= k1 + 1;
  end process;
  -- A register of the groups' values.
  pr : process (clk)
  begin
    if rising_edge(clk) then
      rq <= o1 + x3 + t + w2 + fo;
    end if;
  end process;
  -- Outside the region (`after`): reads a group's value.
  po : process (x3)
  begin
    ob <= x3 after 1 ns;
  end process;
  clocks : process
  begin
    while not done loop
      wait for 5 ns;
      clk <= not clk;
    end loop;
    wait;
  end process;
  main : process
  begin
    wait for 2 ns;
    for i in 1 to 12 loop
      din <= din + 13;
      if i mod 3 = 0 then
        din2 <= din2 + 5;
      end if;
      if i = 5 then
        rst <= '0';
      end if;
      wait for 3 ns;
      report "o1=" & to_hstring(o1) & " x3=" & to_hstring(x3) & " t=" & to_hstring(t) &
        " q=" & to_hstring(q) & " w2=" & to_hstring(w2) & " fo=" & to_hstring(fo) &
        " rq=" & to_hstring(rq) & " ob=" & to_hstring(ob);
      wait for 4 ns;
    end loop;
    done <= true;
    wait;
  end process;
end architecture;
