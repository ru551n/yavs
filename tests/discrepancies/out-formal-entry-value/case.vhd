-- LRM 4.2.2.2 and 14.6 b) 5): the value of an out actual is not copied in; a scalar out formal
-- starts at its subtype's default value ('left) and that value is copied back when the body
-- never assigns it (#1184).
entity top is end entity;
architecture a of top is
  subtype s is integer range 3 to 9;
  procedure rd (variable x : out integer) is begin report "RES x=" & integer'image(x); x := 1; end procedure;
  procedure none (variable x : out s) is begin end procedure;
begin
  process
    variable v : integer := 5;
    variable w : integer range 0 to 100 := 50;
  begin
    rd(v);
    none(w);
    report "RES v=" & integer'image(v) & " w=" & integer'image(w);
    wait;
  end process;
end architecture;
