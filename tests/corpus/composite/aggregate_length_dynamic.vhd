-- difftest: top=aggregate_length_dynamic std=2008 exit=1 ignore=fatalmsg
-- A positional aggregate longer than the (non-static) subtype it is assigned to: a length
-- error (LRM 9.3.3.3), not a write outside the aggregate.
entity aggregate_length_dynamic is end;
architecture t of aggregate_length_dynamic is
  type int_vector is array (natural range <>) of integer;
begin
  process
    variable n : natural := 2;
    variable v : int_vector(1 to n);
  begin
    report "before";
    v := (1, 2, 3);
    report "unreachable";
    wait;
  end process;
end;
