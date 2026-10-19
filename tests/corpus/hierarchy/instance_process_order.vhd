-- difftest: top=instance_process_order std=2008 stop=12ns
-- A process with one wait statement in an instance runs before a process with two wait
-- statements of the architecture that instantiates it, in the same cycle (#252): testbenches
-- that check a value the instance reports in the same cycle depend on it.
entity instance_process_order_dut is port (aclk : in bit); end;
architecture a of instance_process_order_dut is
begin
  process begin
    wait until aclk = '1';
    report "dut edge";
  end process;
end;

entity instance_process_order is end;
architecture a of instance_process_order is
  signal clk : bit := '0';
begin
  clk <= not clk after 5 ns;
  process begin
    wait until clk = '1';
    report "tb edge";
    wait;
  end process;
  dut : entity work.instance_process_order_dut port map (aclk => clk);
end;
