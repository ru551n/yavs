-- difftest: top=range_record_runtime_2019 std=2019 ref=hand
-- ref=hand: LCS-2016-099: an object of a range record type whose value is known only at run
-- time (a constant of a generic, a variable, a signal, a parameter) is a range in an index
-- constraint, a range constraint, an array type, a loop and a slice. No reference simulator
-- runs it (L112); the expected output was checked by hand against the LRM: a subtype or
-- type declaration evaluates its range once, at its elaboration (LRM 14.4.2.1).
entity range_record_runtime_2019 is
  generic (n : integer := 5);
end entity;
architecture t of range_record_runtime_2019 is
  subtype ir is integer'range'record;
  signal s : bit_vector(n downto 0);
  constant rg : ir := s'range'value;
  signal x : bit_vector(rg);
  subtype gs is integer range rg;
  signal sr : ir := (left => 2, right => 0, direction => descending);

  function sum(r : ir) return integer is
    variable v : bit_vector(r);
    variable acc : integer := 0;
  begin
    for i in r loop
      acc := acc * 10 + i;
    end loop;
    return acc + v'length * 1000;
  end function;
begin
  process
    variable rv : ir := (left => 1, right => 4, direction => ascending);
    subtype vs is integer range rv;
    type va is array (rv) of bit;
    variable v : bit_vector(rv);
    variable w : bit_vector(7 downto 0) := "10110010";
    variable a : va;
    variable m : integer := 0;
  begin
    report "x " & integer'image(x'left) & " " & integer'image(x'right) & " " & boolean'image(x'ascending);
    report "gs " & integer'image(gs'low) & " " & integer'image(gs'high);
    report "v " & integer'image(v'left) & " " & integer'image(v'right) & " " & boolean'image(v'ascending);
    for i in rv loop
      m := m * 10 + i;
    end loop;
    for i in v'reverse_range loop
      m := m * 10 + i;
    end loop;
    report "m " & integer'image(m);
    -- The declarations keep the bounds they were elaborated with.
    rv := (left => 6, right => 3, direction => descending);
    report "vs " & integer'image(vs'low) & " " & integer'image(vs'high) & " " & boolean'image(vs'ascending);
    report "va " & integer'image(a'left) & " " & integer'image(a'length);
    report "slice " & to_string(w(rv));
    report "sum " & integer'image(sum(rv)) & " " & integer'image(sum((left => 0, right => 2, direction => ascending)));
    m := 0;
    for i in sr loop
      m := m * 10 + i;
    end loop;
    report "sr " & integer'image(m);
    sr <= (left => 3, right => 5, direction => ascending);
    wait for 1 ns;
    m := 0;
    for i in sr loop
      m := m * 10 + i;
    end loop;
    report "sr " & integer'image(m);
    assert x'length = n + 1 severity failure;
    wait;
  end process;
end architecture;
