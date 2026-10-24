-- difftest: top=idle_vars std=2008
-- Pure processes with variables that are assigned on every path before anything reads them (in
-- the arms of an `if` or `case`, inside a branch, the fields of a record variable one by one),
-- which a wake with no event on a signal they read leaves suspended; and the ones that must run
-- every time: a variable assigned in one arm only and read after the `if`, one assigned in a loop
-- and read after it, a record variable read as a whole after some of its fields were assigned.
-- A `report` under a condition of a generic that is false in an instance does not stop the skip
-- there (u0), and where it is true (u1) every report must come. Every value, time and delta a
-- skipped run would have made must be there.
entity dbg is
  generic (DEBUG : boolean);
  port (clk : in bit; a, b : in natural; o : out natural);
end entity;

architecture t of dbg is
begin
  -- Woken by clk and b, reads a only.
  p : process (clk, a, b)
    variable v : natural;
  begin
    v := a + 1;
    if rising_edge(clk) and DEBUG then
      report "dbg v=" & integer'image(v) severity note;
    end if;
    o <= v;
  end process;
end architecture;

entity idle_vars is end;

architecture t of idle_vars is
  type rec_t is record
    a, b, c : natural;
  end record;
  signal clk : bit := '0';
  signal r : rec_t := (0, 0, 0);
  signal x, y, z : natural := 0;
  signal sel : natural range 0 to 2 := 0;
  signal o1, o2, o3, o4, o5, o6, o7, o8, o9 : natural := 0;
  signal ro : rec_t := (0, 0, 0);
  signal done : boolean := false;
begin
  u0 : entity work.dbg generic map (DEBUG => false) port map (clk, x, y, o8);
  u1 : entity work.dbg generic map (DEBUG => true) port map (clk, x, y, o9);

  clk <= not clk after 5 ns when not done;

  -- Defined in both arms of an if/else, read after it. Only r.a, x and y are read.
  p1 : process (r, x, y, z)
    variable v : natural;
  begin
    if y > 2 then
      v := r.a + x;
    else
      v := x;
    end if;
    o1 <= v * 2;
  end process;

  -- Defined and read inside a branch only, and a second variable defined at the top.
  p2 : process (r, x, y, z)
    variable t, u : natural;
  begin
    u := r.b;
    if y > 1 then
      t := u + y;
      o2 <= t + x;
    else
      o2 <= u;
    end if;
  end process;

  -- Defined in every alternative of a case.
  p3 : process (r, x, sel, z)
    variable v : natural;
  begin
    case sel is
      when 0 => v := r.a;
      when 1 => v := r.b + 1;
      when others => v := x;
    end case;
    o3 <= v;
  end process;

  -- Defined in one arm only and read after the if: the value of the last activation survives,
  -- so every run counts.
  p4 : process (r, y)
    variable v : natural := 0;
  begin
    if y > 2 then
      v := r.c;
    end if;
    o4 <= v;
  end process;

  -- Assigned in a loop, read after it: not defined when the loop runs no iteration.
  p5 : process (r, x, y)
    variable v : natural := 0;
  begin
    for i in 1 to y loop
      v := r.c + i;
    end loop;
    o5 <= v;
  end process;

  -- Defined before a loop, updated and read inside it.
  p6 : process (r, x, y, z)
    variable s : natural;
  begin
    s := r.a;
    for i in 1 to 3 loop
      s := s + x;
    end loop;
    o6 <= s;
  end process;

  -- The fields of a record variable assigned one by one, then read by field.
  p7 : process (r, x, y, z)
    variable w : rec_t;
  begin
    w.a := r.a;
    w.b := x + 1;
    w.c := w.a + w.b;
    o7 <= w.c;
  end process;

  -- Fields b and c keep the value of an earlier activation when y is low: the whole record is
  -- read, so every run counts.
  p8 : process (r, x, y, z)
    variable w : rec_t := (0, 0, 0);
  begin
    w.a := r.a;
    if y > 2 then
      w.b := x;
    end if;
    ro <= w;
  end process;

  mon : process (o1, o2, o3, o4, o5, o6, o7, o8, o9, ro)
  begin
    report "o=" & integer'image(o1) & "," & integer'image(o2) & "," & integer'image(o3) & ","
      & integer'image(o4) & "," & integer'image(o5) & "," & integer'image(o6) & ","
      & integer'image(o7) & "," & integer'image(o8) & "," & integer'image(o9) & " ro="
      & integer'image(ro.a) & "," & integer'image(ro.b);
  end process;

  -- Changes of unread signals (z, most of r and x), of read ones, and writes of the same value.
  stim : process
  begin
    for i in 1 to 30 loop
      wait until clk = '1';
      case i is
        when 2  => z <= 1;                     -- read by nobody
        when 4  => r.c <= 3;                   -- p4, p5 read it
        when 6  => x <= 2;
        when 8  => r.a <= 7;
        when 10 => y <= 3;
        when 12 => z <= 2; r.b <= 4;
        when 14 => sel <= 1;
        when 16 => r <= (a => 7, b => 4, c => 3);   -- no event: same values
        when 18 => sel <= 2; z <= 5;
        when 20 => y <= 1; x <= 6;
        when 22 => r.a <= 8; r.c <= 9;
        when 24 => y <= 0;
        when 26 => sel <= 0; x <= 6;           -- no event on x
        when others => null;
      end case;
    end loop;
    wait for 20 ns;
    done <= true;
    wait;
  end process;
end architecture;
