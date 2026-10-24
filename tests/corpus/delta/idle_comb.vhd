-- difftest: top=idle_comb std=2008
-- Pure processes left suspended when none of the signals they read had an event since their last
-- run (idle skip): combinational processes whose sensitivity list is wider than what they read,
-- ones with a variable assigned before it is read, clocked concurrent assignments, a clocked
-- process with such a variable, and a process that keeps a value in a variable (not pure). Every
-- value, time and delta a skipped run would have made must be there.
entity idle_comb is end;

architecture t of idle_comb is
  type rec_t is record
    a, b, c : natural;
  end record;
  signal clk : bit := '0';
  signal r : rec_t := (0, 0, 0);
  signal x, y : natural := 0;
  signal en : bit := '0';
  signal o1, o2, o3, o4, o5, o6, o7 : natural := 0;
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;

  -- Wider sensitivity than reads: only r.a matters.
  p1 : process (r, x)
  begin
    o1 <= r.a + 1;
  end process;

  -- A variable assigned in full before anything reads it.
  p2 : process (r, x, y)
    variable v : natural;
  begin
    v := r.b + x;
    o2 <= v * 2;
  end process;

  -- Clocked concurrent assignments, with the edge spelled two ways.
  o3 <= r.c when rising_edge(clk);
  o4 <= o1 + o2 when clk'event and clk = '1';

  -- A clocked process with a variable assigned before it is read.
  p5 : process (clk)
    variable v : natural;
  begin
    if rising_edge(clk) then
      v := o3 + x;
      if en = '1' then
        o5 <= v;
      end if;
    end if;
  end process;

  -- A value kept in a variable: every run counts, none may be skipped.
  p6 : process (r)
    variable n : natural := 0;
  begin
    n := n + 1;
    o6 <= n;
  end process;

  -- A variable assigned in one branch only: read before assigned in the other.
  p7 : process (r, y)
    variable v : natural := 0;
  begin
    if y > 2 then
      v := r.c;
    end if;
    o7 <= v;
  end process;

  mon : process (o1, o2, o3, o4, o5, o6, o7)
  begin
    report "o=" & integer'image(o1) & "," & integer'image(o2) & "," & integer'image(o3) & ","
      & integer'image(o4) & "," & integer'image(o5) & "," & integer'image(o6) & ","
      & integer'image(o7);
  end process;

  stim : process
  begin
    for i in 1 to 24 loop
      wait until rising_edge(clk);
      case i is
        when 2  => r.c <= 5;                  -- unread by p1, p2; read by o3
        when 4  => r.b <= 3;                  -- p2
        when 6  => x <= 2;                    -- p2 (and p5 at the next edge)
        when 8  => r.a <= 9;                  -- p1
        when 10 => y <= 4;                    -- p7 reads r.c now
        when 12 => en <= '1';
        when 14 => r.c <= 7; y <= 1;          -- p7 keeps its variable
        when 16 => r <= (a => 9, b => 3, c => 7);  -- no event: same values
        when 18 => x <= 2;                    -- no event
        when 20 => r.a <= 10; r.b <= 4;
        when 22 => en <= '0';
        when others => null;
      end case;
    end loop;
    wait for 20 ns;
    done <= true;
    wait;
  end process;
end architecture;
