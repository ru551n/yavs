-- difftest: top=sig_param_range_forward2 std=2008 exit=1 ref=hand
-- ref=hand: sim1 and sim2 accept the value (discrepancy `sig-param-range`). The formal passed
-- on through two procedures still carries the bounds of the first actual (LRM 14.7.3, #1181).
entity sig_param_range_forward2 is end entity;
architecture a of sig_param_range_forward2 is
  signal s : integer range 0 to 9 := 0;
  procedure leaf (signal o : out integer; v : integer) is begin o <= v; end procedure;
  procedure mid (signal o : out integer; v : integer) is begin leaf(o, v); end procedure;
begin
  process
  begin
    mid(s, 10);
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
