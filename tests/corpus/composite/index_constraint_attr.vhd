-- difftest: top=index_constraint_attr exit=1 ref=hand
-- ref=hand: the expected text is yavs's wording of the run-time error (the references word it
-- differently).
-- An index constraint given by an attribute range must also lie inside the index subtype of
-- the array type (LRM 5.3.2.2): string(x'range) with x'range = 0 to 3 has the bound 0 outside
-- POSITIVE, so the declaration of s fails at run time (#637). A valid attribute range and a
-- null one elaborate without an error.
entity index_constraint_attr is end;
architecture a of index_constraint_attr is
  procedure foo(x : bit_vector) is
    variable s : string(x'range);
  begin
    report "foo len " & integer'image(s'length) & " left " & integer'image(s'left);
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
