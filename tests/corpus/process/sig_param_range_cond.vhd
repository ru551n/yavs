-- difftest: top=sig_param_range_cond std=2008 exit=1 ref=hand
-- ref=hand: sim1 and sim2 accept the value (discrepancy `sig-param-range`). A conditional
-- assignment through a signal formal (LRM 14.7.3, #1181).
entity sig_param_range_cond is end entity;
architecture a of sig_param_range_cond is
  signal s : integer range 0 to 9 := 0;
  procedure cond (signal o : out integer; c : boolean) is begin o <= 10 when c else 1; end procedure;
begin
  process
  begin
    cond(s, true);
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
