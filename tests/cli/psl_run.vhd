entity psl_run is end;
architecture t of psl_run is
  signal clk, ok : bit;
  default clock is clk'event and clk = '1';
  property never_true is always false;
begin
  clk <= not clk after 1 ns when now < 10 ns;
  -- psl assert always false;
  assert always false;
  cover {clk; ok};
  process
  begin
    wait for 5 ns;
    report "design with PSL ran";
    wait;
  end process;
end;

vunit psl_checks (psl_run) {
  assert never ok;
}
