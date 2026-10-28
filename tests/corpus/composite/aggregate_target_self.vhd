-- difftest: top=aggregate_target_self std=2008 ref=hand
-- An aggregate target whose value is one of its own variables (#815): the value is read
-- whole before any element is written (LRM 10.6.2.1). ref=hand: the first reference
-- simulator writes element by element from the partly written variable; the second
-- prints the values below.
entity aggregate_target_self is end;
architecture t of aggregate_target_self is
  type rec is record a, b, c, d : integer; end record;
  type iv is array (0 to 3) of integer;
  function img(r : rec) return string is
  begin
    return integer'image(r.a) & integer'image(r.b) & integer'image(r.c) & integer'image(r.d);
  end;
begin
  process
    variable r1, r2 : rec := (1, 2, 3, 4);
    variable v : iv := (1, 2, 3, 4);
    variable s : bit_vector(0 to 3) := "1100";
  begin
    (r1.d, r1.c, r1.b, r1.a) := r1;
    (d => r2.a, c => r2.b, b => r2.c, a => r2.d) := r2;
    report img(r1) & " " & img(r2);
    (v(3), v(2), v(1), v(0)) := v;
    report integer'image(v(0)) & integer'image(v(1)) & integer'image(v(2)) & integer'image(v(3));
    (s(3), s(2), s(1), s(0)) := s;
    report to_string(s);
    wait;
  end process;
end;
