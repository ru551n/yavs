-- difftest: top=aggregate_named_pieces std=2008
-- 2008 array-valued elements under range choices in named aggregates (#230): logic,
-- integer and character arrays, arrays of words and of records, with `others`, element
-- choices and range choices of elements, choices against the aggregate's direction, bounds
-- from the choices and a non-static choice. Named bounds take the direction of a
-- constrained target (LRM 9.3.3.3).
entity aggregate_named_pieces is end;
architecture t of aggregate_named_pieces is
  type iv is array (natural range <>) of integer;
  type w8 is array (natural range <>) of bit_vector(7 downto 0);
  type pt is record x : integer; en : boolean; end record;
  type pts is array (natural range <>) of pt;
  signal s : bit_vector(7 downto 0);
  function img(v : iv) return string is
  begin
    if v'length = 0 then return ""; end if;
    return integer'image(v(v'left)) & "," & img(v(v'left + 1 to v'right));
  end;
  function lr(v : iv) return string is
  begin
    return integer'image(v'left) & ":" & integer'image(v'right);
  end;
begin
  process
    variable a : bit_vector(3 downto 0) := "1100";
    variable b : bit_vector(0 to 1) := "01";
    variable v : bit_vector(7 downto 0);
    variable u : bit_vector(0 to 7);
    variable x : iv(0 to 1) := (1, 2);
    variable y : iv(0 to 5);
    variable str : string(1 to 6);
    variable m2 : w8(0 to 1) := (x"01", x"02");
    variable m : w8(0 to 3);
    variable p2 : pts(0 to 1) := ((7, true), (8, false));
    variable p : pts(0 to 2);
    variable k : natural := 3;
    variable o : bit := '1';
    variable w : iv(3 downto 0);
    variable pd : pts(2 downto 0);
  begin
    v := (7 downto 4 => a, others => '0');
    report to_string(v);
    v := (0 to 3 => a, others => '0');
    u := (7 downto 4 => a, 1 downto 0 => b, others => '1');
    report to_string(v) & " " & to_string(u);
    u := (0 to 3 => a, 4 => '1', 5 to 6 => b, 7 => '0');
    v := (7 downto 6 => b, 5 downto 4 => '1', 3 downto 0 => a);
    report to_string(u) & " " & to_string(v);
    y := (4 to 5 => x, 0 to 1 => x, others => 9);
    report img(y) & " " & img(iv'(2 to 3 => x, 4 => 5)) & " " & lr(iv'(2 to 3 => x, 4 => 5));
    str := (1 to 2 => "ab", 3 => 'c', 4 to 6 => "def");
    report str;
    m := (2 to 3 => m2, others => x"ff");
    report to_string(m(0)) & " " & to_string(m(2)) & " " & to_string(m(3));
    p := (1 to 2 => p2, 0 => (3, true));
    report integer'image(p(0).x) & integer'image(p(1).x) & integer'image(p(2).x)
      & boolean'image(p(2).en);
    report img(iv'(k to k + 1 => x)) & " " & lr(iv'(k to k + 1 => x));
    -- Named bounds take the direction of a constrained target.
    v := (7 => o, 6 downto 0 => '0');
    w := (3 => k, 2 downto 0 => 0);
    pd := (2 => (k, true), 1 downto 0 => (5, false));
    report to_string(v) & " " & integer'image(w(3)) & integer'image(pd(2).x)
      & integer'image(pd(0).x);
    pd := (1 downto 0 => p2, 2 => (k, true));
    report integer'image(pd(2).x) & integer'image(pd(1).x) & integer'image(pd(0).x);
    s <= (7 downto 4 => a, 3 downto 0 => not a);
    wait for 1 ns;
    report to_string(s);
    assert v = "10000000" and y(5) = 2 and str = "abcdef" severity failure;
    wait;
  end process;
end;
