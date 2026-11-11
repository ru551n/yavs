-- Records with unconstrained fields read in functions on native frames: each `LayoutOf` of a
-- run-time descriptor there is one call of the layout memo, not a step of the host.
entity layout_into is generic (N : positive := 3); end;
architecture a of layout_into is
  type rec is record
    a : integer_vector;
    b : string;
    k : integer;
  end record;
  type recs is array (natural range <>) of rec;
  function f(r : rec) return integer is
  begin
    return r.a(r.a'left) * 3 + r.b'length + r.a'length + r.k + character'pos(r.b(r.b'right));
  end function;
  function g(v : recs) return integer is
    variable s : integer := 0;
  begin
    for i in v'range loop
      s := (s * 7 + f(v(i))) mod 99991;
    end loop;
    return s;
  end function;
begin
  process
    variable r1 : rec(a(1 to 3), b(1 to 2));
    variable r2 : rec(a(0 to 4), b(1 to 7));
    variable v : recs(1 to 2)(a(1 to 2), b(1 to 3));
    variable s : integer := 0;
    variable q : rec(a(1 to N), b(1 to N + 1));
  begin
    for k in 0 to 99 loop
      r1 := ((k, 2, 3), "xy", k);
      r2 := ((-k, 1, 2, 3, 4), "abcdefg", 2 * k);
      v := (((k, 1), "abc", 1), ((2, k), "def", k));
      q.a(1 + k mod N) := k;
      q.k := q.a(1) + q.b'length;
      s := (s * 31 + f(r1) + f(r2) + g(v) + q.k) mod 99991;
      wait for 1 ns;
    end loop;
    report "s=" & integer'image(s);
    wait;
  end process;
end;
