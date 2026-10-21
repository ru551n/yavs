-- difftest: top=multi_process_order std=2008 ignore=order
-- p1 and p3 resume at 2 ns in the same cycle. The LRM leaves their order open; sim1's comes from
-- its event heap and yavs's (elaboration order) is deterministic, so compare per process.
entity multi_process_order is end;
architecture t of multi_process_order is
begin
  p1 : process begin wait for 1 ns; report "p1 start"; wait for 1 ns; report "p1 end"; wait; end process;
  p2 : process begin wait for 1 ns; report "p2 start"; wait for 2 ns; report "p2 end"; wait; end process;
  p3 : process begin report "p3 at 0"; wait for 2 ns; report "p3 at 2"; wait; end process;
end;
