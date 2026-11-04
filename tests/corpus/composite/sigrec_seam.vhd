-- difftest: top=sigrec_seam
-- Whole record signals assigned and read many times with mostly the same value (#1050): a gather
-- must see every change of a leaf, a scatter of a value equal to the last one must still do what
-- an assignment does around force, release, delayed entries, other assignments of the same
-- activation, a resolved leaf and observers of the transactions.
library ieee;
use ieee.std_logic_1164.all;

entity sigrec_seam is
end entity;

architecture t of sigrec_seam is
  type rec_t is record
    a : std_ulogic;
    b : std_ulogic_vector(7 downto 0);
    c : std_logic;
    n : integer;
    w : std_ulogic_vector(99 downto 0);
    f : boolean;
  end record;
  constant init : rec_t := ('0', x"00", '0', 0, (others => '0'), false);

  function sum(x : rec_t) return integer is
    variable s : integer := x.n mod 1000;
  begin
    if x.a = '1' then s := s + 1; end if;
    if x.c = '1' then s := s + 2; elsif x.c = 'Z' then s := s + 4; elsif x.c = 'X' then s := s + 8; end if;
    for i in x.b'range loop
      s := s * 3 mod 65521;
      if x.b(i) = '1' then s := s + 1; end if;
    end loop;
    if x.w(99) = '1' then s := s + 16; end if;
    if x.w(0) = '1' then s := s + 32; end if;
    if x.f then s := s + 64; end if;
    return s;
  end function;

  signal clk : bit := '0';
  signal step : integer := 0;
  signal r, rin : rec_t := init;      -- two-process register
  signal q : rec_t := init;           -- several assignments in one activation
  signal d : rec_t := init;           -- delayed assignments
  signal m : rec_t := init;           -- resolved leaf c, two drivers
  signal o : rec_t := init;           -- observed transactions
  signal fo : rec_t := init;          -- forced, read back by a process that reassigns
  signal trans, active_o, events_o, dv : integer := 0;
begin
  clk <= not clk after 5 ns when step < 120;

  count : process (clk)
  begin
    if clk = '1' then step <= step + 1; end if;
  end process;

  comb : process (r, step)
    variable v : rec_t;
  begin
    v := r;
    if step mod 16 = 3 then v.b(step mod 8) := not r.b(step mod 8); end if;
    if step mod 32 = 7 then v.w(99) := not r.w(99); end if;
    if step mod 40 = 9 then v.n := r.n + 1; end if;
    if step mod 24 = 11 then v.a := not r.a; v.f := not r.f; end if;
    rin <= v;
  end process;

  reg : process (clk)
  begin
    if clk = '1' then r <= rin; end if;
  end process;

  -- The same value, then other assignments of the same activation over it, in both orders.
  several : process (clk)
    variable v : rec_t := init;
  begin
    if clk = '1' then
      if step mod 20 = 5 then v.b := x"5A"; end if;
      if step mod 20 = 15 then v.b := x"A5"; end if;
      if step mod 7 = 1 then
        q <= v; q.a <= '1';
      elsif step mod 7 = 2 then
        q.a <= '1'; q <= v;
      elsif step mod 7 = 3 then
        q <= v; q.b(2) <= not v.b(2); q.b(2) <= v.b(2);
      elsif step mod 7 = 4 then
        q <= init; q <= v;
      else
        q <= v;
      end if;
    end if;
  end process;

  -- The same value at no delay while a later entry waits, and a delayed one of the same value.
  delayed : process (clk)
    variable v : rec_t := init;
  begin
    if clk = '1' then
      if step mod 13 = 0 then v.b := not v.b; end if;
      if step mod 11 = 4 then
        d <= v after 2 ns;
        d.a <= '1' after 7 ns;
      elsif step mod 11 = 5 then
        d <= v;
      elsif step mod 11 = 6 then
        d <= reject 1 ns inertial v after 3 ns;
      else
        d <= v;
      end if;
    end if;
  end process;

  -- Leaf c has a second source for a while: the value the first driver repeats is not the signal's.
  m1 : process (clk)
    variable v : rec_t := init;
  begin
    if clk = '1' then
      v.c := '1';
      m <= v;
    end if;
  end process;

  m2 : process (step)
  begin
    if step mod 30 = 12 then m.c <= '0'; elsif step mod 30 = 20 then m.c <= 'Z'; end if;
  end process;

  observed : process (clk)
    variable v : rec_t := init;
  begin
    if clk = '1' then
      if step mod 17 = 8 then v.a := not v.a; end if;
      o <= v;
    end if;
  end process;

  trans <= trans + 1 when o.a'transaction'event;
  count_active : process (o.b'transaction)
  begin
    if o.b'active then active_o <= active_o + 1; end if;
  end process;
  events_o <= events_o + 1 when o.a'event;

  -- Reads its own forced value and assigns it back, unchanged, while a force holds.
  back : process (clk)
    variable v : rec_t;
  begin
    if clk = '1' then
      v := fo;
      if step mod 18 = 2 then v.b := not v.b; end if;
      fo <= v;
      dv <= sum(fo'driving_value);
    end if;
  end process;

  force_it : process
  begin
    wait until step = 30;
    fo.b <= force x"C3";
    wait until step = 40;
    fo.b <= release;
    wait until step = 50;
    fo.w <= force (others => '1');
    r.b <= force x"7E";
    wait until step = 60;
    fo.w <= release;
    r.b <= release;
    wait until step = 70;
    fo.b <= force fo.b;
    wait until step = 75;
    fo.b <= release;
    wait;
  end process;

  show : process (clk)
  begin
    if clk = '0' then
      report integer'image(step) & " r=" & integer'image(sum(r)) & " q=" & integer'image(sum(q))
        & " d=" & integer'image(sum(d)) & " m=" & integer'image(sum(m)) & " o=" & integer'image(sum(o))
        & " fo=" & integer'image(sum(fo)) & " t=" & integer'image(trans)
        & " e=" & integer'image(events_o) & " a=" & integer'image(active_o) & " dv=" & integer'image(dv);
    end if;
  end process;
end architecture;
