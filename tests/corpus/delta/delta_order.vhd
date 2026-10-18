-- difftest: top=delta_order std=2008
entity delta_order is end;
architecture t of delta_order is
  signal a, b, c : integer := 0;
begin
  process begin
    a <= 1;
    wait for 0 ns;
    report "after zero wait a=" & integer'image(a);
    wait for 1 ns;
    a <= 2; b <= a;
    wait for 0 ns;
    report "a=" & integer'image(a) & " b=" & integer'image(b);
    wait;
  end process;
  process (a) begin c <= a; end process;
  process (c) begin report "c=" & integer'image(c) & " @" & time'image(now); end process;
end;
