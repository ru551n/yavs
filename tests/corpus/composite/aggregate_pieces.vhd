-- difftest: top=aggregate_pieces std=2008
-- 2008 array-valued elements in positional aggregates (#89): slices mixed with elements of
-- logic, integer and character arrays, of arrays of words and arrays of records, as values
-- and as variable and signal aggregate targets.
entity aggregate_pieces is end;
architecture t of aggregate_pieces is
  type iv is array (natural range <>) of integer;
  type w8 is array (natural range <>) of bit_vector(7 downto 0);
  type ia is array (1 to 3) of integer;
  type iaa is array (natural range <>) of ia;
  type pt is record x : integer; en : boolean; end record;
  type pts is array (natural range <>) of pt;
  signal sa, sb : bit_vector(1 to 3);
  signal sv : bit_vector(5 downto 0);
  signal si : iv(0 to 3);
  signal i0, i1 : iv(0 to 1);
  signal ic : integer;
  signal iw : iv(0 to 2);
  function img(v : iv) return string is
  begin
    if v'length = 0 then return ""; end if;
    return integer'image(v(v'left)) & "," & img(v(v'left + 1 to v'right));
  end;
begin
  process
    variable a : bit_vector(3 downto 0) := "1010";
    variable b : bit_vector(3 downto 0) := "0101";
    variable v : bit_vector(7 downto 0);
    variable w : bit_vector(9 downto 0);
    variable x : iv(0 to 1) := (1, 2);
    variable y : iv(0 to 2);
    variable z : iv(0 to 0);
    variable e : iv(0 to -1);
    variable s1 : string(1 to 2);
    variable s2 : string(1 to 3);
    variable m : w8(0 to 2);
    variable m2 : w8(0 to 1) := (x"01", x"02");
    variable q : iaa(0 to 2);
    variable q2 : iaa(0 to 1) := ((1, 2, 3), (4, 5, 6));
    variable p : pts(0 to 2);
    variable p2 : pts(0 to 1) := ((7, true), (8, false));
  begin
    v := (a, b);
    w := (a, '1', b, '0');
    report to_string(v) & " " & to_string(w);
    y := (x, 3);
    report img(y) & " " & img((x, e, x, 9));
    (y, z) := iv'(5, 6, 7, 8);
    report img(y) & " " & img(z);
    (s1, s2) := string'("hello");
    report s1 & "|" & s2 & "|" & string'("ab", "cd", "ef");
    m := (m2, x"ff");
    report to_string(m(0)) & " " & to_string(m(1)) & " " & to_string(m(2));
    m := (x"aa", m2(1 to 1), m2(0 to 0));
    report to_string(m(0)) & " " & to_string(m(1)) & " " & to_string(m(2));
    q := ((9, 9, 9), q2);
    report integer'image(q(0)(3)) & integer'image(q(1)(1)) & integer'image(q(2)(3));
    p := (p2, (3, true));
    report integer'image(p(0).x) & integer'image(p(1).x) & integer'image(p(2).x)
      & boolean'image(p(2).en) & boolean'image(p(1).en);
    sv <= ("101", "011");
    si <= (x, x);
    wait for 1 ns;
    (sa, sb) <= sv;
    (i0, ic, i1) <= iv'(1, 2, 3, 4, 5);
    wait for 1 ns;
    iw <= (ic, i0) after 2 ns;
    report to_string(sv) & " " & to_string(sa) & " " & to_string(sb) & " " & img(si);
    report img(i0) & " " & integer'image(ic) & " " & img(i1);
    wait for 3 ns;
    report img(iw);
    wait;
  end process;
end;
