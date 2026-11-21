-- difftest: top=sig_param_range_forward std=2008 exit=1 ref=hand
-- ref=hand: sim1 and sim2 accept the value (discrepancy `sig-param-range`). A signal formal
-- passed on to a procedure with a wider formal: the value still belongs to the subtype of
-- the first actual, and to that of the formal passed on (LRM 14.7.3, #1181).
entity sig_param_range_forward is end entity;
architecture a of sig_param_range_forward is
  signal s : integer range 0 to 9 := 0;
  signal w : integer range -5 to 20 := 0;
  procedure leaf (signal o : out integer; v : integer) is begin o <= v; end procedure;
  procedure mid (signal o : out natural; v : integer) is begin leaf(o, v); end procedure;
  procedure outer (signal o : out integer; v : integer) is begin mid(o, v); end procedure;
begin
  process
  begin
    outer(s, 9);
    mid(w, 15);
    wait for 1 ns;
    report "s=" & integer'image(s) & " w=" & integer'image(w);
    mid(w, -1);
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
