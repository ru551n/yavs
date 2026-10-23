-- difftest: top=record_aggregate_field_length exit=1
-- A record aggregate whose array element has another length than the record field's, whose
-- bounds come from a generic, is a length error even when the aggregate is locally static
-- (LRM 9.3.3.2, 5.3.2.2, #463). The aggregate used to be folded and copied into the field.
entity record_aggregate_field_length is
  generic (n : natural := 5);
end;
architecture a of record_aggregate_field_length is
  type rec is record
    s : string(1 to n);
    k : integer;
  end record;
  constant good : string(1 to 5) := "hello";
  constant bad : string(1 to 3) := "abc";
begin
  process
    variable v : rec;
  begin
    v := (s => good, k => 2);
    report "good " & v.s & integer'image(v.k);
    v := (s => bad, k => 3);
    report "not reached";
    wait;
  end process;
end;
