-- difftest: top=nested_instances std=2008
entity inv is port (a : in bit; y : out bit); end;
architecture a of inv is begin y <= not a after 1 ns; end;
entity chain is port (i : in bit; o : out bit); end;
architecture a of chain is
  signal m : bit;
begin
  u1 : entity work.inv port map (i, m);
  u2 : entity work.inv port map (m, o);
end;
entity nested_instances is end;
architecture t of nested_instances is
  signal i, o : bit := '0';
begin
  dut : entity work.chain port map (i, o);
  process (o) begin report "o=" & bit'image(o) & " @" & time'image(now); end process;
  process begin wait for 10 ns; i <= '1'; wait for 10 ns; i <= '0'; wait; end process;
end;
