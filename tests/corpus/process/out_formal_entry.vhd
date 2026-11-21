-- difftest: top=out_formal_entry std=2008
-- An out formal passed through a temporary (a narrower actual, a 4-byte array element) reads the
-- actual's value on entry and leaves it unchanged when never assigned, as one passed by
-- reference does; the copy in is dropped only where the body assigns the formal first (#1175).
-- sim2 starts an out formal at its subtype's 'left instead (#1184).
entity out_formal_entry is end entity;
architecture a of out_formal_entry is
  procedure rd (variable x : out integer) is begin report "rd x=" & integer'image(x); x := 1; end procedure;
  procedure none (variable x : out integer) is begin end procedure;
  procedure set (variable x : out integer; v : integer) is begin x := v; end procedure;
  procedure setd (variable x : out integer; v : integer) is
    variable t : integer := 3;
  begin
    x := v + t;
  end procedure;
begin
  process
    type ia is array (0 to 3) of integer;
    variable a : ia := (others => 42);
    variable w : integer range 0 to 100 := 50;
    variable i : integer := 2;
  begin
    rd(w);
    report "w=" & integer'image(w);
    w := 50;
    none(w);
    report "w=" & integer'image(w);
    rd(a(i));
    report "a=" & integer'image(a(i));
    a(i) := 42;
    none(a(i));
    report "a=" & integer'image(a(i));
    set(a(i), 7);
    setd(a(1), 7);
    set(w, 9);
    report "a=" & integer'image(a(i)) & " " & integer'image(a(1)) & " w=" & integer'image(w);
    wait;
  end process;
end architecture;
