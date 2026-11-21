-- difftest: top=sig_param_range std=2008 exit=1 ref=hand
-- ref=hand: sim1 and sim2 drive 10 into the actual with no check (discrepancy
-- `sig-param-range`). A value driven through an out or inout signal formal is the driving
-- value of the actual, so it belongs to the actual's subtype (LRM 14.7.3, #1181): values in
-- range go through, 10 does not fit `integer range 0 to 9`.
entity sig_param_range is end entity;
architecture a of sig_param_range is
  signal s : integer range 0 to 9 := 0;
  signal n : natural := 0;
  procedure drv (signal o : out integer; v : integer) is begin o <= v; end procedure;
  procedure drn (signal o : out natural; v : natural) is begin o <= v; end procedure;
begin
  process
  begin
    drv(s, 7);
    drn(n, 12);
    wait for 1 ns;
    report "s=" & integer'image(s) & " n=" & integer'image(n);
    drv(s, 10);
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
