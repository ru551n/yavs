-- difftest: top=record_aggregate_field_range exit=1 ref=hand
-- ref=hand: the first reference simulator does not check it (the second fails, as the LRM says),
-- and yavs words the error "value V outside of INTEGER range L to H".
-- A record aggregate whose scalar element lies outside the field's range, which depends on a
-- generic, fails when evaluated even when the aggregate is locally static (LRM 9.3.3.2, 5.2.1,
-- #463).
entity record_aggregate_field_range is
  generic (n : natural := 5);
end;
architecture a of record_aggregate_field_range is
  type rec is record
    k : integer range 0 to n;
  end record;
begin
  process
    variable v : rec;
  begin
    v := (k => 5);
    report "good " & integer'image(v.k);
    v := (k => 6);
    report "not reached";
    wait;
  end process;
end;
