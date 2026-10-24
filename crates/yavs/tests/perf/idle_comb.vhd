-- Pure processes that a wake leaves suspended when none of the signals they read had an event
-- (idle skip): combinational processes with a sensitivity list wider than what they read (one
-- with a variable assigned before it is read), clocked concurrent assignments, a clocked process
-- with such a variable, a pure function declared in the architecture, and two processes that must
-- run every time (a value kept in a variable, a variable read before it is assigned). 600 cycles;
-- the record changes in `a` every 50 edges, in `c` every 70, `x` every 90.
library ieee; use ieee.std_logic_1164.all;
entity idle_comb is end;
architecture a of idle_comb is
  type rec_t is record
    a, b, c : natural range 0 to 255;
  end record;
  signal clk : std_logic := '0';
  signal r : rec_t := (0, 0, 0);
  signal x, y : natural range 0 to 255 := 0;
  signal en : std_logic := '0';
  signal o1, o2, o3, o4, o5, o6, o7, o8 : natural := 0;
  signal done : boolean := false;
  function twice(v : natural) return natural is
  begin
    return 2 * v;
  end function;
begin
  clk <= not clk after 5 ns when not done;
  p1 : process (r, x) begin o1 <= r.a + 1; end process;
  p2 : process (r, x, y)
    variable v : natural;
  begin
    v := r.b + x;
    o2 <= twice(v);
  end process;
  o3 <= r.c when rising_edge(clk);
  o4 <= o1 + o2 when clk'event and clk = '1';
  p5 : process (clk)
    variable v : natural;
  begin
    if rising_edge(clk) then
      v := o3 + x;
      if en = '1' then o5 <= v; end if;
    end if;
  end process;
  p6 : process (r)
    variable n : natural := 0;
  begin
    n := (n + 1) mod 256;
    o6 <= n;
  end process;
  p7 : process (r, y)
    variable v : natural := 0;
  begin
    if y > 2 then v := r.c; end if;
    o7 <= v;
  end process;
  p8 : process (r, x, y) begin o8 <= r.b + y; end process;
  stim : process begin
    for i in 1 to 600 loop
      wait until rising_edge(clk);
      if i mod 50 = 0 then r.a <= (r.a + 7) mod 256; end if;
      if i mod 70 = 0 then r.c <= (r.c + 3) mod 256; end if;
      if i mod 90 = 0 then x <= (x + 1) mod 256; end if;
      if i mod 110 = 0 then y <= (y + 1) mod 5; end if;
      if i = 100 then en <= '1'; end if;
      if i = 450 then en <= '0'; end if;
    end loop;
    report "o=" & integer'image(o1) & "," & integer'image(o2) & "," & integer'image(o3) & ","
      & integer'image(o4) & "," & integer'image(o5) & "," & integer'image(o6) & ","
      & integer'image(o7) & "," & integer'image(o8);
    done <= true; wait;
  end process;
end;
