-- difftest: top=sig_param_range_concurrent std=2008 exit=1 ref=hand
-- ref=hand: sim1 and sim2 accept the value (discrepancy `sig-param-range`). A concurrent
-- procedure call is a process calling the procedure: the value driven through the formal
-- belongs to the actual's subtype (LRM 14.7.3, #1181).
entity sig_param_range_concurrent is end entity;
architecture a of sig_param_range_concurrent is
  signal s : integer range 0 to 9 := 0;
  signal go : integer := 3;
  procedure drv (signal o : out integer; signal v : in integer) is begin o <= v; end procedure;
begin
  drv(s, go);
  process
  begin
    wait for 1 ns;
    report "s=" & integer'image(s);
    go <= 10;
    wait for 1 ns;
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
