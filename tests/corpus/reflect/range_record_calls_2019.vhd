-- difftest: top=range_record_calls_2019 std=2019 ref=hand
-- ref=hand: LCS-2016-099: any expression of a range record type is a range, a function call
-- too (index constraint, range constraint, loop, slice, generate), a qualified expression, a
-- parenthesised call and a user operator. sim1 and sim2 reject the form (L112); the
-- expected output was checked by hand against the LRM: the range is
-- evaluated once where it is elaborated (a declaration, LRM 14.4.2.1; a loop, 10.10).
-- A variable of an anonymous scalar subtype (`integer range counted(5)`) is left out: its
-- range is evaluated at every check (#1174).
entity range_record_calls_2019 is
  generic (n : integer := 5);
end entity;
architecture t of range_record_calls_2019 is
  subtype ir is integer'range'record;
  type ra is array (natural range <>) of ir;

  function up(a, b : integer) return ir is
  begin
    return (left => a, right => b, direction => ascending);
  end function;

  function dn(k : integer) return ir is
  begin
    return (left => k, right => 0, direction => descending);
  end function;

  -- A call with a range record argument stays a call.
  function ones(r : ir) return bit_vector is
    variable b : bit_vector(r) := (others => '1');
  begin
    return b;
  end function;

  -- The result of a parameterless function, sliced by a call.
  -- A user operator returning a range record: the reversed range.
  function "-"(r : ir) return ir is
  begin
    if r.direction = ascending then
      return (left => r.right, right => r.left, direction => descending);
    end if;
    return (left => r.right, right => r.left, direction => ascending);
  end function;

  function pat return bit_vector is
  begin
    return "1100";
  end function;

  signal x : bit_vector(dn(n));
  subtype gs is integer range up(2, n);
  constant tbl : ra(0 to 1) := (up(1, 3), dn(2));
  signal y : bit_vector(x'range'value);
  signal sg : bit_vector(0 to 2) := "000";
  type bm is array (integer range <>, integer range <>) of bit;
  type bva is array (integer range <>) of bit_vector;
  type rt is record
    a : bit_vector;
    b : integer;
  end record;
  subtype idx is integer range 0 to 2;
begin
  g : for i in up(0, 1) generate
    sg(i) <= '1';
  end generate;

  process
    variable calls : integer := 0;
    impure function counted(k : integer) return ir is
    begin
      calls := calls + 1;
      return (left => 0, right => k, direction => ascending);
    end function;
    variable k : integer := 3;
    variable v : bit_vector(up(1, 4));
    subtype ps is integer range up(k, 9);
    variable w : bit_vector(7 downto 0) := "10110010";
    variable m : integer := 0;
    variable e : bit_vector(tbl(0));
    variable mm : bm(up(0, 1), dn(2));
    variable ev : bva(up(1, 2))(dn(3));
    type ta is array (dn(1)) of bit;
    variable tv : ta;
    -- A record constraint keeps its meaning.
    variable r1 : rt(a(idx));
    variable r2 : rt(a(up(4, 7)));
    -- Each declaration calls once, at its elaboration; uses and checks do not call again.
    subtype cs is integer range counted(4);
    variable cv : bit_vector(counted(2));
  begin
    report "decl " & integer'image(calls);
    m := cv'length + cs'high;
    cv := "101";
    report "uses " & integer'image(calls) & " " & integer'image(m);
    assert calls = 2 severity failure;
    m := 0;
    report "x " & integer'image(x'left) & " " & integer'image(x'right) & " " & boolean'image(x'ascending);
    report "y " & integer'image(y'left) & " " & integer'image(y'right);
    report "gs " & integer'image(gs'low) & " " & integer'image(gs'high);
    report "v " & integer'image(v'left) & " " & integer'image(v'right) & " " & boolean'image(v'ascending);
    for i in dn(k) loop
      m := m * 10 + i;
    end loop;
    for i in tbl(0) loop
      m := m * 10 + i;
    end loop;
    report "m " & integer'image(m);
    m := 0;
    for i in counted(k) loop
      m := m * 10 + i;
      k := 0;
    end loop;
    report "counted " & integer'image(m) & " " & integer'image(calls);
    assert m = 123 and calls = 3 severity failure;
    -- `ps` keeps the bounds of its elaboration (k was 3), now that k is 0.
    report "ps " & integer'image(ps'low) & " " & integer'image(ps'high);
    k := 3;
    report "cslice " & to_string(sg(counted(1))) & " " & integer'image(calls);
    assert calls = 4 severity failure;
    report "slice " & to_string(w(dn(k))) & " " & to_string(w(tbl(1))) & " " & to_string(w(x'range'value));
    report "ones " & to_string(ones(dn(2))) & " pat " & to_string(pat(up(1, 2)));
    m := 0;
    for i in ir'(left => 2, right => 3, direction => ascending) loop
      m := m * 10 + i;
    end loop;
    for i in (dn(1)) loop
      m := m * 10 + i;
    end loop;
    for i in -up(5, 6) loop
      m := m * 10 + i;
    end loop;
    report "expr " & integer'image(m) & " " & to_string(w((dn(1))));
    report "e " & integer'image(e'left) & " " & integer'image(e'right);
    report "mm " & integer'image(mm'left(1)) & " " & integer'image(mm'right(1)) & " " & integer'image(mm'left(2)) & " " & integer'image(mm'right(2));
    report "ev " & integer'image(ev'left) & " " & integer'image(ev'right) & " " & integer'image(ev(1)'left) & " " & integer'image(ev(1)'right);
    report "ta " & integer'image(tv'left) & " " & integer'image(tv'right);
    report "rt " & integer'image(r1.a'left) & " " & integer'image(r1.a'right) & " " & integer'image(r2.a'left) & " " & integer'image(r2.a'right);
    wait for 0 ns;
    report "sg " & to_string(sg);
    wait;
  end process;
end architecture;
