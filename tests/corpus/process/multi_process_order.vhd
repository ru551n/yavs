-- difftest: top=multi_process_order std=2008
entity multi_process_order is end;
architecture t of multi_process_order is
begin
  p1 : process begin wait for 1 ns; report "p1 start"; wait for 1 ns; report "p1 end"; wait; end process;
  p2 : process begin wait for 1 ns; report "p2 start"; wait for 2 ns; report "p2 end"; wait; end process;
  p3 : process begin report "p3 at 0"; wait for 2 ns; report "p3 at 2"; wait; end process;
end;
