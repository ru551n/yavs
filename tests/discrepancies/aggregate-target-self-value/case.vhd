-- (r.d, r.c, r.b, r.a) := r: the value is read whole before the first element is written
-- (LRM 10.6.2.1), so the record is reversed.
entity top is end;
architecture a of top is
  type rec is record a, b, c, d : integer; end record;
begin
  process
    variable r : rec := (1, 2, 3, 4);
  begin
    (r.d, r.c, r.b, r.a) := r;
    report "RES " & integer'image(r.a) & integer'image(r.b) & integer'image(r.c) & integer'image(r.d);
    wait;
  end process;
end;
