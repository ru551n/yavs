-- difftest: top=real_array_eq std=2008
-- #1144: = and /= on composites holding reals compare by value (-0.0 = 0.0), not by bytes.
-- Ordering operators are not defined for arrays of reals (LRM 9.2.3), so there are none here.
entity real_array_eq is end;
architecture t of real_array_eq is
  type rr is record x : real; i : integer; end record;
  type rr_array is array (natural range <>) of rr;
  type rv_rec is record v : real_vector(0 to 1); n : integer; end record;
  type rvv is array (0 to 1) of real_vector(0 to 1);
  type r2 is array (0 to 1, 0 to 1) of real;
  subtype my_real is real range -10.0 to 10.0;
  type my_vec is array (natural range <>) of my_real;
begin
  process
    variable z : real := 0.0;
    variable a, b, c : real_vector(0 to 1);
    variable n1, n2 : real_vector(1 to 0);
    variable l3 : real_vector(0 to 2) := (0.0, 1.0, 2.0);
    variable ma, mb : my_vec(0 to 1);
    variable ra, rb : rr_array(0 to 1);
    variable va, vb : rv_rec;
    variable xa, xb : rvv;
    variable ta, tb : r2;
  begin
    z := -z;  -- negative zero
    a := (0.0, 1.0);
    b := (z, 1.0);
    c := (0.0, 2.0);
    report "vector eq " & boolean'image(a = b) & " ne " & boolean'image(a /= b);
    report "vector rev " & boolean'image(b = a) & " ne " & boolean'image(b /= a);
    report "vector same " & boolean'image(b = b) & " ne " & boolean'image(b /= b);
    report "vector diff " & boolean'image(a = c) & " ne " & boolean'image(a /= c);
    report "vector slice " & boolean'image(a = l3(0 to 1)) & " len " & boolean'image(a = l3);
    report "vector null " & boolean'image(n1 = n2) & " ne " & boolean'image(n1 /= n2);
    report "vector lit " & boolean'image(b = (0.0, 1.0)) & " " & boolean'image((z, 1.0) = a);
    ma := (0.0, 1.0);
    mb := (z, 1.0);
    report "subtype eq " & boolean'image(ma = mb) & " ne " & boolean'image(ma /= mb);
    ra(0) := (x => 0.0, i => 1); ra(1) := (x => 2.0, i => 3);
    rb(0) := (x => z, i => 1);   rb(1) := (x => 2.0, i => 3);
    report "records eq " & boolean'image(ra = rb) & " ne " & boolean'image(ra /= rb);
    rb(1).i := 4;
    report "records diff " & boolean'image(ra = rb) & " ne " & boolean'image(ra /= rb);
    va := (v => (0.0, 1.0), n => 5);
    vb := (v => (z, 1.0), n => 5);
    report "vrec eq " & boolean'image(va = vb) & " ne " & boolean'image(va /= vb);
    vb.n := 6;
    report "vrec diff " & boolean'image(va = vb) & " ne " & boolean'image(va /= vb);
    xa := ((0.0, 1.0), (2.0, 3.0));
    xb := ((z, 1.0), (2.0, 3.0));
    report "nested eq " & boolean'image(xa = xb) & " ne " & boolean'image(xa /= xb);
    xb(1)(1) := 4.0;
    report "nested diff " & boolean'image(xa = xb) & " ne " & boolean'image(xa /= xb);
    ta := ((0.0, 1.0), (2.0, 3.0));
    tb := ((z, 1.0), (2.0, 3.0));
    report "2d eq " & boolean'image(ta = tb) & " ne " & boolean'image(ta /= tb);
    tb(1, 0) := -2.0;
    report "2d diff " & boolean'image(ta = tb) & " ne " & boolean'image(ta /= tb);

    wait;
  end process;
end;
