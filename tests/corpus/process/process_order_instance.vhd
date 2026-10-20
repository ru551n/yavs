-- difftest: top=process_order_instance std=2008
-- Processes with several waits that resume on the same clock edge after running before: a
-- process of an instance declared after the testbench process reports before it, like one of an
-- instance declared before it (the references that agree run instance processes before the
-- parent's); the parent's own processes keep declaration order (process/process_order_resumed).
-- A testbench that tests a flag a checker inside an instance sets on that edge depends on it
-- (#469).
entity checker is
  generic (nm : string);
  port (clk : bit);
end;
architecture t of checker is
begin
  p : process begin
    wait until clk = '1';
    loop
      wait until clk = '1';
      report nm;
    end loop;
  end process;
end;

entity process_order_instance is end;
architecture t of process_order_instance is
  signal clk : bit := '0';
begin
  clk <= not clk after 5 ns;
  i0 : entity work.checker generic map ("instance before main") port map (clk);
  m0 : process begin
    wait until clk = '1';
    loop
      wait until clk = '1';
      report "monitor before main";
    end loop;
  end process;
  main : process begin
    wait until clk = '1';
    wait until clk = '1';
    for k in 1 to 4 loop
      wait until clk = '1';
      report "main";
    end loop;
    wait for 1 ns;
    std.env.finish;
  end process;
  m1 : process begin
    wait until clk = '1';
    loop
      wait until clk = '1';
      report "monitor after main";
    end loop;
  end process;
  i1 : entity work.checker generic map ("instance after main") port map (clk);
end;
