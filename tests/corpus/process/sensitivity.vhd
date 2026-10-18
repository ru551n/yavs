-- difftest: top=sensitivity std=2008
entity sensitivity is end;
architecture t of sensitivity is
  signal x, y, z : integer := 0;
begin
  process (x, y) begin
    report "sens x=" & integer'image(x) & " y=" & integer'image(y) & " @" & time'image(now);
  end process;
  process (all) begin
    z <= x + y;
  end process;
  process (z) begin
    report "z=" & integer'image(z) & " @" & time'image(now);
  end process;
  stim : process begin
    wait for 1 ns; x <= 1;
    wait for 1 ns; y <= 2;
    wait for 1 ns; x <= 1; -- no event
    wait for 1 ns; x <= 5; y <= 5;
    wait;
  end process;
end;
