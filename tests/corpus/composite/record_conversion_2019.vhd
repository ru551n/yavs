-- difftest: top=tb_record_conversion_2019 std=2019
-- 2019 (LCS-2016-075): a conversion between record types with the same element names converts
-- each element: reordered elements, nested records, arrays of the same type, integer to real.
entity tb_record_conversion_2019 is end;
architecture t of tb_record_conversion_2019 is
  type t_rec1 is record x, y : integer; end record;
  type t_rec2 is record y, x : integer; end record;
  type t_rec3 is record
    x : t_rec1;
    y : integer_vector(1 to 3);
    z : integer;
  end record;
  type t_rec4 is record
    y : integer_vector;
    x : t_rec2;
    z : real;
  end record;
begin
  process
    variable r1 : t_rec1;
    variable r2 : t_rec2;
    variable r3 : t_rec3;
    variable r4 : t_rec4(y(1 to 3));
  begin
    r1 := (1, 2);
    r2 := t_rec2(r1);
    assert r2 = (2, 1);
    r3 := (x => (4, 5), y => (1, 2, 3), z => 9);
    r4 := t_rec4(r3);
    assert r4.x = (5, 4);
    assert r4.y = (1, 2, 3);
    assert r4.z = 9.0;
    r4 := ((7, 8, 9), (6, 5), 1.0);
    r3 := t_rec3(r4);
    assert r3.x = (5, 6);
    assert r3.y = (7, 8, 9);
    assert r3.z = 1;
    report "ok";
    wait;
  end process;
end;
