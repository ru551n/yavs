-- difftest: top=concurrent_aggregate_choices
-- A concurrent assignment is sensitive to signals read in aggregate choices (LRM 11.6, 10.2) (#374).
entity concurrent_aggregate_choices is end;
architecture a of concurrent_aggregate_choices is
  signal i : natural := 1;
  signal s : bit_vector(1 to 3);
begin
  s <= (1 to i => '1') & (i + 1 to 3 => '0');
  process
  begin
    i <= 2;
    wait for 1 ns;
    assert s = "110" report "s = " & to_string(s) severity failure;
    report "s = " & to_string(s);
    wait;
  end process;
end;
