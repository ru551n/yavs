-- difftest: top=sig_param_range_time std=2008 exit=1 ref=hand
-- ref=hand: sim1 and sim2 accept the value (discrepancy `sig-param-range`). A physical actual
-- narrower than the formal (LRM 14.7.3, #1181).
entity sig_param_range_time is end entity;
architecture a of sig_param_range_time is
  subtype short is time range 0 ns to 5 ns;
  signal t : short := 0 ns;
  procedure dt (signal o : out time; v : time) is begin o <= v; end procedure;
begin
  process
  begin
    dt(t, 6 ns);
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
