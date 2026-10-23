-- Delayed assignments to idle drivers (inertial and transport) behind a clocked toggle: scalar
-- quick transactions in the future queue.
entity delayed is end;
architecture a of delayed is
  signal clk : bit := '0';
  signal a1, b1, c1 : bit := '0';
  signal nb, nc : integer := 0;
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  process (clk) begin
    if clk = '1' then a1 <= not a1 after 1 ns; end if;
  end process;
  b1 <= a1 after 2 ns;
  c1 <= transport b1 after 3 ns;
  process (b1) begin nb <= nb + 1; end process;
  process (c1) begin nc <= nc + 1; end process;
  process begin
    wait for 1010 ns;
    report "a=" & bit'image(a1) & " nb=" & integer'image(nb) & " nc=" & integer'image(nc);
    done <= true; wait;
  end process;
end;
