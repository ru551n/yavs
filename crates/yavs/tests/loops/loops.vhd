library ieee; use ieee.std_logic_1164.all;
entity loops is end;
architecture a of loops is
  type slm is array (natural range <>, natural range <>) of std_logic;
  function img(v : std_logic_vector) return string is
    variable s : string(1 to v'length);
    variable j : natural := 1;
  begin
    for i in v'range loop s(j) := std_logic'image(v(i))(2); j := j + 1; end loop;
    return s;
  end function;
  function copy(a : std_logic_vector) return std_logic_vector is
    variable r : std_logic_vector(a'range);
  begin
    for i in a'range loop r(i) := a(i); end loop;
    return r;
  end function;
  function rev(a : std_logic_vector) return std_logic_vector is
    variable r : std_logic_vector(a'range);
  begin
    for i in a'range loop r(i) := a(a'high + a'low - i); end loop;
    return r;
  end function;
  function fill(n : natural; c : std_logic) return std_logic_vector is
    variable r : std_logic_vector(n - 1 downto 0) := (others => '0');
  begin
    for i in r'range loop r(i) := c; end loop;
    return r;
  end function;
  function evens(a : std_logic_vector) return std_logic_vector is
    variable r : std_logic_vector(a'range) := (others => '-');
  begin
    for i in 0 to a'length / 2 - 1 loop r(i * 2) := a(i); end loop;
    return r;
  end function;
  function row(m : slm; k : natural) return std_logic_vector is
    variable r : std_logic_vector(m'range(2));
  begin
    for i in r'range loop r(i) := m(k, i); end loop;
    return r;
  end function;
  function over(a : std_logic_vector; n : natural) return std_logic_vector is
    variable r : std_logic_vector(a'range) := a;
  begin
    for i in 0 to n loop r(i) := a(i); end loop;
    return r;
  end function;
  function shl1(a : std_logic_vector) return std_logic_vector is
    variable r : std_logic_vector(a'range) := (others => '0');
  begin
    for i in a'high downto a'low + 1 loop r(i) := a(i - 1); end loop;
    return r;
  end function;
  function col(m : slm; k : natural) return std_logic_vector is
    variable r : std_logic_vector(m'range(1));
  begin
    for i in m'range(1) loop r(i) := m(i, k); end loop;
    return r;
  end function;
  procedure setrow(m : inout slm; k : natural; a : std_logic_vector) is
  begin
    for i in a'range loop m(k, i) := a(i); end loop;
  end procedure;
  function part(a : std_logic_vector; lo : natural; hi : natural) return std_logic_vector is
    variable r : std_logic_vector(hi downto lo);
  begin
    for i in lo to hi loop r(i) := a(i); end loop;
    return r;
  end function;
  procedure shift(v : inout std_logic_vector) is
  begin
    for i in v'low to v'high - 1 loop v(i + 1) := v(i); end loop;
  end procedure;
  procedure shiftdown(v : inout std_logic_vector) is
  begin
    for i in v'low to v'high - 1 loop v(i) := v(i + 1); end loop;
  end procedure;
begin
  process
    variable c70 : std_logic_vector(0 to 69) :=
      "1100110011001100110011001100110011001100110011001100110011001100110011";
    variable v : std_logic_vector(15 downto 0) := "10XZ01LH-WU1100X";
    variable w : std_logic_vector(0 to 15);
    variable m : slm(0 to 2, 7 downto 0) := (others => (others => '0'));
    variable u : std_logic_vector(7 downto 0);
    variable o : std_logic_vector(10 to 25) := "1X0Z1L0H1-0U1W01";
    variable wide : std_logic_vector(129 downto 0);
    variable m2 : slm(1 to 3, 0 to 7) := (others => (others => '1'));
    variable lowv : std_logic_vector(7 downto 0);
  begin
    report "copy " & img(copy(v));
    report "copy70 " & img(copy(c70));
    report "rev " & img(rev(v));
    report "fill " & img(fill(40, '1'));
    report "evens " & img(evens(v));
    for k in 0 to 2 loop
      for j in 7 downto 0 loop m(k, j) := v(j + k); end loop;
    end loop;
    report "row1 " & img(row(m, 1));
    report "row2 " & img(row(m, 2));
    report "over " & img(over(v, 15));
    w := v; shift(w); report "shift " & img(w);
    w := v; shiftdown(w); report "shiftdown " & img(w);
    u := (others => '0');
    for i in 3 to 3 loop u(i) := '1'; end loop;
    for i in 3 to 4 loop u(i) := 'H'; end loop;
    for i in 7 downto 5 loop u(i) := 'L'; end loop;
    for i in 4 to 2 loop u(i) := 'X'; end loop;
    report "short " & img(u);
    report "shl1 " & img(shl1(v)) & " " & img(shl1(o));
    report "col " & img(col(m, 3)) & " " & img(col(m, 0));
    report "copy16 " & img(copy(o));
    for i in wide'range loop wide(i) := c70(i mod 70); end loop;
    report "copy130 " & img(copy(wide)) & " " & img(rev(wide)) & " " & img(shl1(wide));
    lowv := v(7 downto 0);
    setrow(m2, 2, lowv);
    lowv := v(15 downto 8);
    setrow(m2, 3, lowv);
    report "setrow " & img(row(m2, 1)) & img(row(m2, 2)) & img(row(m2, 3));
    report "part " & img(part(v, 3, 9)) & " " & img(part(wide, 60, 70)) & " " & img(part(wide, 0, 129));
    report "part0 " & img(part(v, 5, 4));
    report "over " & img(over(v, 16));
    wait;
  end process;
end;

