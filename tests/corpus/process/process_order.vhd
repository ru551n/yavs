-- difftest: top=process_order std=2008 ref=hand
-- Order of processes within one cycle (#252, docs/decisions.md "Process order within a
-- cycle"). The LRM leaves it open; yavs's rule: at initialisation elaboration order; after
-- that the processes with one suspension point (a sensitivity list, or exactly one wait
-- statement and no wait in a called procedure) in elaboration order, then the others in
-- reverse elaboration order, those resumed by a signal before those resumed by a timeout.
-- ref=hand: the reference simulator runs the processes of the 1 ns cycle in another order,
-- so the .expected follows yavs's rule.
entity process_order_leaf is port (go : in bit); end;
architecture a of process_order_leaf is
begin
  lm : process begin wait on go; report "u.lm two waits"; wait; end process;
  ls : process (go) begin if now > 0 ns then report "u.ls sensitivity list"; end if; end process;
end;

entity process_order is end;
architecture t of process_order is
  signal go : bit;
begin
  go <= '1' after 1 ns;
  m1 : process begin report "m1 init"; wait on go; report "m1 event"; wait; end process;
  t1 : process begin report "t1 init"; wait for 1 ns; report "t1 timeout"; wait; end process;
  s1 : process (go) begin if now > 0 ns then report "s1 sensitivity list"; end if; end process;
  u : entity work.process_order_leaf port map (go);
  w1 : process begin wait on go; report "w1 one wait"; end process;
  m2 : process begin report "m2 init"; wait on go; report "m2 event"; wait; end process;
  t2 : process begin report "t2 init"; wait for 1 ns; report "t2 timeout"; wait; end process;
  pr : process
    procedure wait_go is begin wait on go; end procedure;
  begin
    wait_go; report "pr wait in a procedure"; wait;
  end process;
end;
