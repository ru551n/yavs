-- difftest: top=named_return_record_2019 std=2019
-- 2019 named return values (4.2.1) of a record type with an unconstrained field: each call
-- takes the result subtype from its target (variables, constants, signals, signal initial
-- values and a concurrent assignment). Generic subprograms and protected methods are in
-- `named_return_generic_2019` (the reference simulator fails on them).
entity named_return_record_2019 is end;
architecture t of named_return_record_2019 is
  type rec_t is record
    f : integer_vector;
    n : natural;
  end record;

  function iota return r of rec_t is
    variable v : r;
    variable k : natural := 0;
  begin
    for i in v.f'range loop
      v.f(i) := k;
      k := k + 1;
    end loop;
    v.n := v.f'length;
    return v;
  end function;

  signal s1 : rec_t(f(1 to 5));
  signal s2 : rec_t(f(6 to 7)) := iota;
  signal s3 : rec_t(f(0 to 0));
begin
  s3 <= iota;

  process
    variable v1 : rec_t(f(1 to 3));
    variable v2 : rec_t(f(7 downto 4));
    constant c1 : rec_t(f(4 to 5)) := iota;
  begin
    v1 := iota;
    report "v1 " & to_string(v1.f(1)) & to_string(v1.f(3)) & " " & to_string(v1.n);
    v2 := iota;
    report "v2 " & to_string(v2.f(7)) & to_string(v2.f(4)) & " " & to_string(v2.n);
    report "c1 " & to_string(c1.f(5)) & " " & to_string(c1.n);
    report "s2 " & to_string(s2.f(7)) & " " & to_string(s2.n);
    s1 <= iota;
    wait for 1 ns;
    report "s1 " & to_string(s1.f(1)) & to_string(s1.f(5)) & " " & to_string(s1.n);
    report "s3 " & to_string(s3.f(0)) & " " & to_string(s3.n);
    wait;
  end process;
end;
