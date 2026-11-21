-- difftest: top=sig_param_range_inout std=2008 exit=1 ref=hand
-- ref=hand: sim1 and sim2 let `o <= o + 1` drive 10 into the actual (discrepancy
-- `sig-param-range`); the driving value of the actual belongs to its subtype (LRM 14.7.3,
-- #1181).
entity sig_param_range_inout is end entity;
architecture a of sig_param_range_inout is
  signal s : integer range 0 to 9 := 8;
  procedure inc (signal o : inout integer) is begin o <= o + 1; end procedure;
begin
  process
  begin
    inc(s);
    wait for 1 ns;
    report "s=" & integer'image(s);
    inc(s);
    wait for 1 ns;
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
