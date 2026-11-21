-- LRM 4.2.2.2: a scalar variable parameter of mode out or inout is passed by copy, so a body
-- that reads the actual by its own name sees the old value until the call returns (#1187).
entity top is end entity;
architecture a of top is
begin
  process
    variable v : integer := 5;
    variable w : integer := 7;
    procedure first (x : out integer) is
    begin
      x := 1;
      report "RES first v=" & integer'image(v);
    end procedure;
    procedure incr (x : inout integer) is
    begin
      x := x + 1;
      report "RES incr w=" & integer'image(w);
    end procedure;
  begin
    first(v);
    incr(w);
    report "RES v=" & integer'image(v) & " w=" & integer'image(w);
    wait;
  end process;
end architecture;
