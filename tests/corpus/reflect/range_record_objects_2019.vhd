-- difftest: top=range_record_objects_2019 std=2019 ref=hand
-- ref=hand: LCS-2016-099: a constant of a range record type (`R'RECORD`) is a range in an
-- index constraint, a range constraint, an array type definition and a loop. Only one of the
-- reference simulators accepts it; the expected output is its result, which agrees with
-- the LRM reading, except `x'ascending`, where it answers true for a descending range (LRM: false).
entity range_record_objects_2019 is end entity;
architecture t of range_record_objects_2019 is
  subtype nrr is natural'range'record;
  signal a : bit_vector(7 downto 0);
  constant r : nrr := a'range'value;
  constant r2 : nrr := (direction => descending, left => 30, right => 0);
  constant up : nrr := (direction => ascending, left => 2, right => 5);
  signal x : bit_vector(r);
  signal y : bit_vector(r2);
  subtype sr is integer range up;
  type arr is array (up) of integer;
  signal z : arr;
begin
  process
    variable n : integer := 0;
  begin
    report "x " & integer'image(x'left) & " " & integer'image(x'right) & " " & boolean'image(x'ascending);
    report "y " & integer'image(y'left) & " " & integer'image(y'length);
    report "sr " & integer'image(sr'low) & " " & integer'image(sr'high);
    report "z " & integer'image(z'left) & " " & integer'image(z'length);
    for i in up loop
      n := n + i;
    end loop;
    for i in r2 loop
      n := n + 1;
    end loop;
    report "sum " & integer'image(n);
    assert r.left = 7 and r.right = 0 and r.direction = descending severity failure;
    wait;
  end process;
end architecture;
