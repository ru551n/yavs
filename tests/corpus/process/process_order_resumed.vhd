-- difftest: top=process_order_resumed std=2008
-- A monitor and a testbench process, each with several waits, resume on the same clock edge
-- after both have run before. The LRM leaves their order open (14.7.5.3); the reference
-- simulators that agree run them in elaboration order here (the monitor, declared first,
-- reports first), unlike the first resumption after initialisation (process/process_order
-- in tests/discrepancies). A testbench that tests a queue the monitor empties on that edge
-- depends on it (#417).
entity process_order_resumed is end;
architecture t of process_order_resumed is
  signal clk : bit := '0';
  signal reset : bit := '1';
  signal rdy : bit := '0';
  signal cnt : integer := 0;
begin
  clk <= not clk after 5 ns;
  dut : process (clk) begin
    if clk = '1' then
      cnt <= cnt + 1;
      if cnt mod 3 /= 0 then rdy <= '1'; else rdy <= '0'; end if;
    end if;
  end process;
  mon : process begin
    wait until reset = '0' and clk = '1';
    loop
      wait until clk = '1';
      report "monitor";
    end loop;
  end process;
  main : process
    procedure drain (n : natural) is begin
      for i in 1 to n loop
        wait until clk = '1';
        report "main drain";
      end loop;
    end;
  begin
    wait until clk = '1';
    wait until clk = '1';
    reset <= '0';
    wait until clk = '1';
    for k in 1 to 4 loop
      wait until clk = '1' and rdy = '1';
      report "main send";
    end loop;
    drain(4);
    std.env.finish;
  end process;
end;
