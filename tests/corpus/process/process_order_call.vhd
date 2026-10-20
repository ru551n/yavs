-- difftest: top=process_order_call std=2008
-- A concurrent procedure call that loops on a wait, declared before a testbench process, reports
-- before it on every clock edge, also after both have run before: a protocol checker written
-- that way has to see the edge before a testbench that reads its log on the same edge does
-- (#462).
entity process_order_call is end;
architecture t of process_order_call is
  signal clk : bit := '0';
  procedure check (signal c : in bit) is
  begin
    loop
      wait until c = '1';
      report "checker";
    end loop;
  end;
begin
  clk <= not clk after 5 ns;
  c0 : check(clk);
  main : process begin
    wait until clk = '1';
    wait for 1 ns;
    for k in 1 to 5 loop
      wait until clk = '1';
      report "main";
    end loop;
    std.env.finish;
  end process;
end;
