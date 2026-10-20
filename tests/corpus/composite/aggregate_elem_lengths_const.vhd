-- difftest: top=aggregate_elem_lengths_const std=2008 exit=2 ref=hand
-- ref=hand: the reference simulator stops in its elaboration step; the expected output is the
-- run-time failure of the constant's initial value.
-- A constant aggregate with elements of different lengths for an unconstrained element
-- subtype fails when the declaration is elaborated (LRM 9.3.3.3, 14.4.2.4).
entity aggregate_elem_lengths_const is end;
architecture t of aggregate_elem_lengths_const is
  type strings is array (natural range <>) of string;
  constant c : strings(0 to 2) := ("one", "three", "ab");
begin
  process
  begin
    report c(0);
    wait;
  end process;
end;
