-- difftest: top=aggregate_target_cover_signal std=2008 exit=1
-- A signal aggregate target with 2008 slice pieces must cover each waveform value exactly
-- (#230, LRM 10.5.2.1): a value one element shorter than the pieces is a runtime error,
-- raised before any piece is driven.
entity aggregate_target_cover_signal is end;
architecture t of aggregate_target_cover_signal is
  signal sa, sb : bit_vector(1 to 2);
begin
  process
    variable v : bit_vector(0 to 3) := "1011";
    variable k : natural := 3;
  begin
    (sa, sb) <= v(0 to k);
    wait for 1 ns;
    report to_string(sa) & " " & to_string(sb);
    k := k - 1;
    (sa, sb) <= v(0 to k);
    wait for 1 ns;
    report "not reached";
    wait;
  end process;
end;
