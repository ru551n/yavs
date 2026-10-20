-- difftest: top=index_constraint_compat exit=1 ref=hand
-- ref=hand: the expected text is yavs's wording of the run-time error (the references word it
-- differently).
-- An index constraint must lie inside the index subtype of the array type (LRM 5.3.2.2); a
-- violation, also of a bound known only at run time, is a run-time error where the object is
-- declared; a null range is always compatible (#379).
entity index_constraint_compat is end;
architecture a of index_constraint_compat is
  function mk(n : integer) return natural is
    variable b : bit_vector(1 downto n);
    variable e : bit_vector(1 to n - 5);
  begin
    return b'length + e'length;
  end function;
begin
  process
  begin
    report "mk(0) " & integer'image(mk(0));
    report "mk(1) " & integer'image(mk(1));
    report "mk(-1) " & integer'image(mk(-1));
    report "not reached";
    wait;
  end process;
end;
