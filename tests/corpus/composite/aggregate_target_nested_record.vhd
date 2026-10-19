-- difftest: top=aggregate_target_nested_record std=2008
-- Nested aggregate targets of records (#350, LRM 10.5.2.1, 10.6.2.1; lrm-checks L98): an
-- aggregate target inside an aggregate target, for variables and for signals, with scalar,
-- array and real fields, whole sub-records taken by one name, and a function result.
entity aggregate_target_nested_record is end entity;
architecture t of aggregate_target_nested_record is
  type inner is record
    a : integer;
    b : boolean;
  end record;
  type vec is array (0 to 1) of integer;
  type outer is record
    i : inner;
    c : integer;
    v : vec;
  end record;
  type deep is record
    o : outer;
    d : real;
  end record;
  function mk(k : integer) return deep is
  begin
    return (((k, k > 1), k + 1, (k + 2, k + 3)), real(k) + 0.5);
  end function;
  signal sa, sc, s0, s1 : integer := 0;
  signal sb : boolean := false;
  signal sd : real := 0.0;
  signal sv : vec;
begin
  process
    variable a, c, v0, v1 : integer;
    variable b : boolean;
    variable d : real;
    variable i : inner;
    variable v : vec;
  begin
    (((a, b), c, (v0, v1)), d) := mk(1);
    assert a = 1 and not b and c = 2 and v0 = 3 and v1 = 4 and d = 1.5 severity failure;
    (((a, b), c, v), d) := mk(2);
    assert a = 2 and b and c = 3 and v(0) = 4 and v(1) = 5 and d = 2.5 severity failure;
    ((i, c, (v0, v1)), d) := mk(3);
    assert i.a = 3 and i.b and c = 4 and v0 = 5 and v1 = 6 and d = 3.5 severity failure;
    ((i, c, v), d) := deep'(((7, false), 8, (9, 10)), 11.5);
    assert i.a = 7 and not i.b and c = 8 and v = (9, 10) and d = 11.5 severity failure;
    (((sa, sb), sc, (s0, s1)), sd) <= mk(4);
    wait for 1 ns;
    assert sa = 4 and sb and sc = 5 and s0 = 6 and s1 = 7 and sd = 4.5 severity failure;
    (((sa, sb), sc, sv), sd) <= mk(5) after 1 ns;
    wait for 2 ns;
    assert sa = 5 and sb and sc = 6 and sv = (7, 8) and sd = 5.5 severity failure;
    report "a=" & integer'image(a) & " sa=" & integer'image(sa) & " sc=" & integer'image(sc);
    wait;
  end process;
end architecture;
