-- difftest: top=sig_param_range_real std=2008 exit=1 ref=hand
-- ref=hand: sim1 and sim2 accept the value (discrepancy `sig-param-range`). A floating-point
-- actual narrower than the formal (LRM 14.7.3, #1181).
entity sig_param_range_real is end entity;
architecture a of sig_param_range_real is
  subtype unit is real range 0.0 to 1.0;
  signal u : unit := 0.0;
  procedure du (signal o : out real; v : real) is begin o <= v; end procedure;
begin
  process
  begin
    du(u, 2.0);
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
