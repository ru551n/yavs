-- difftest: top=index_constraint_elem exit=1 ref=hand
-- ref=hand: sim1 does not check these constraints (the second reference simulator does); the
-- expected text is yavs's wording of the run-time error.
-- The index constraint of a record element or of an array element subtype must lie inside the
-- index subtype too (LRM 5.3.2.2, #637): with x'range = 0 to 3, string(x'range) has the bound
-- 0 outside POSITIVE, so the declaration fails at run time. A valid and a null range elaborate.
entity index_constraint_elem is end;
architecture a of index_constraint_elem is
  type arr is array (natural range <>) of string;
  procedure foo(x : bit_vector) is
    type rec is record
      f : string(x'reverse_range);
    end record;
    variable r : rec;
    variable e : arr(0 to 1)(x'range);
  begin
    report "foo len " & integer'image(r.f'length) & " " & integer'image(e(0)'length);
  end procedure;
begin
  process
    variable good : bit_vector(1 to 4);
    variable null_r : bit_vector(1 to 0);
    variable bad : bit_vector(0 to 3);
  begin
    foo(good);
    foo(null_r);
    foo(bad);
    report "not reached";
    wait;
  end process;
end;
