-- difftest: top=array_eq_bytes std=2008
-- `=` and `/=` of one-dimensional arrays of enumeration and integer elements: equal and
-- different contents, different lengths, null arrays, slices with other bounds, a string
-- constant and a computed one.
entity array_eq_bytes is
end entity;

architecture a of array_eq_bytes is
  type int_arr is array (natural range <>) of integer range 0 to 1000;
  type color is (red, green, blue);
  type color_arr is array (natural range <>) of color;
begin
  process
    constant c1 : string := "hello world";
    variable s  : string(1 to 11) := "hello world";
    variable t  : string(5 downto 1) := "abcde";
    variable n1 : string(1 to 0);
    variable n2 : string(3 to 2);
    variable i1 : int_arr(1 to 4) := (1, 2, 3, 4);
    variable i2 : int_arr(10 to 13) := (1, 2, 3, 4);
    variable i3 : int_arr(1 to 4) := (1, 2, 3, 5);
    variable k1 : color_arr(0 to 2) := (red, green, blue);
    variable k2 : color_arr(0 to 2) := (red, green, green);
    variable f  : string(1 to 5);
  begin
    assert c1 = s;
    assert not (c1 /= s);
    assert s(1 to 5) = "hello";
    assert s(7 to 11) /= "hello";
    assert s(1 to 4) /= "hello";
    assert not (s(1 to 4) = "hello");
    assert n1 = n2;
    assert not (n1 /= n2);
    assert n1 /= "x";
    assert t = "abcde";
    assert t /= "abcdf";
    assert i1 = i2;
    assert i1 /= i3;
    assert i1(1 to 3) = i3(1 to 3);
    assert k1 /= k2;
    assert k1(0 to 1) = k2(0 to 1);
    f := s(1 to 5);
    assert f = "hello" and f /= "hellp";
    report "ok";
    wait;
  end process;
end architecture;
