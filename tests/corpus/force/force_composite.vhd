-- difftest: top=force_composite std=2008
-- Force and release of composite signals and their parts (#68): every scalar subelement of the
-- target is forced (LRM 10.5.2.1), in the simple and conditional forms.
library ieee; use ieee.std_logic_1164.all;
entity force_composite is end entity;
architecture a of force_composite is
  type rec is record a : std_logic; b : std_logic_vector(7 downto 0); n : integer; end record;
  type recs is array (0 to 1) of rec;
  type ints is array (1 to 3) of integer;
  signal x : std_logic_vector(7 downto 0) := x"0f";
  signal r : rec := ('0', x"11", 1);
  signal rs : recs := (others => ('0', x"00", 0));
  signal v : ints := (1, 2, 3);
begin
  process
    variable sel : integer := 2;
  begin
    wait for 1 ns;
    x <= force not x;
    r <= force ('1', x"a5", 7);
    v <= force (7, 8, 9);
    wait for 1 ns;
    report "x=" & to_hstring(x) & " r.a=" & std_logic'image(r.a) & " r.b=" & to_hstring(r.b)
      & " r.n=" & integer'image(r.n) & " v(2)=" & integer'image(v(2));
    assert x = x"f0" and r = ('1', x"a5", 7) and v = (7, 8, 9) severity failure;
    -- Assignments while forced change the driving value only.
    x <= x"33"; r.n <= 100; v(1) <= 50;
    wait for 1 ns;
    assert x = x"f0" and r.n = 7 and v(1) = 7 severity failure;
    -- Release of a part: that part goes back to its driving value.
    x(3 downto 0) <= release;
    r.n <= release;
    wait for 1 ns;
    report "x=" & to_hstring(x) & " r.n=" & integer'image(r.n);
    assert x = x"f3" and r.n = 100 severity failure;
    x <= release; r <= release; v <= release;
    wait for 1 ns;
    report "x=" & to_hstring(x) & " r.b=" & to_hstring(r.b) & " v(1)=" & integer'image(v(1));
    assert x = x"33" and r = ('0', x"11", 100) and v = (50, 2, 3) severity failure;
    -- Slices, record fields and elements of arrays of records.
    x(7 downto 4) <= force "1010";
    r.b(3 downto 0) <= force x"c";
    rs(1) <= force ('1', x"77", 5);
    rs(0).b <= force x"ee";
    wait for 1 ns;
    report "x=" & to_hstring(x) & " r.b=" & to_hstring(r.b) & " rs(1).b=" & to_hstring(rs(1).b)
      & " rs(0).b=" & to_hstring(rs(0).b);
    assert x = x"a3" and r.b = x"1c" and rs(1) = ('1', x"77", 5) and rs(0) = ('0', x"ee", 0)
      severity failure;
    rs(0) <= release; rs(1) <= release; x <= release; r.b <= release;
    wait for 1 ns;
    assert rs = recs'(others => ('0', x"00", 0)) and x = x"33" and r.b = x"11" severity failure;
    -- The conditional form.
    x <= force x"01" when sel = 1 else x"02" when sel = 2 else x"03";
    r <= force ('1', x"20", 2) when sel = 2 else ('0', x"30", 3);
    wait for 1 ns;
    report "x=" & to_hstring(x) & " r.b=" & to_hstring(r.b);
    assert x = x"02" and r = ('1', x"20", 2) severity failure;
    x <= release; r <= release;
    wait for 1 ns;
    assert x = x"33" and r = ('0', x"11", 100) severity failure;
    report "done";
    wait;
  end process;
end architecture;
