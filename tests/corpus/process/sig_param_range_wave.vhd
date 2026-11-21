-- difftest: top=sig_param_range_wave std=2008 exit=1 ref=hand
-- ref=hand: sim1 and sim2 accept the value (discrepancy `sig-param-range`). Each element of
-- the waveform is checked at the assignment, as for a signal of the actual's subtype: the
-- error comes at 0 ns, before the transaction of 10 at 5 ns could mature or be preempted
-- (LRM 14.7.3, #1181).
entity sig_param_range_wave is end entity;
architecture a of sig_param_range_wave is
  signal s : integer range 0 to 9 := 0;
  procedure drv (signal o : out integer) is begin o <= 1, 2 after 1 ns, 10 after 5 ns; end procedure;
begin
  process
  begin
    report "start";
    drv(s);
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
