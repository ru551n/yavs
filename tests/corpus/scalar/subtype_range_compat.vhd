-- difftest: top=subtype_range_compat exit=1 ref=hand
-- ref=hand: the expected text is yavs's wording of the run-time error (the references word it
-- differently, and three of them do not check a range known only at run time).
-- A range constraint must lie inside the range of its type mark (LRM 5.2.1, 6.4.2.3); a
-- violation, also of a bound known only at run time, is a run-time error where the object is
-- declared (#379).
entity subtype_range_compat is end;
architecture a of subtype_range_compat is
  function first(n : integer) return integer is
    variable v : positive range n to 3 := 3;
    subtype s is natural range n - 1 to 3;
    variable w : s := 3;
  begin
    return v + w;
  end function;
begin
  process
    variable ok : positive range 1 to 3 := 2;
  begin
    report "ok " & integer'image(ok);
    report "first(2) " & integer'image(first(2));
    report "first(3) " & integer'image(first(3));
    report "first(0) " & integer'image(first(0));
    report "not reached";
    wait;
  end process;
end;
