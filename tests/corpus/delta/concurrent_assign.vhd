-- difftest: top=concurrent_assign std=2008
entity concurrent_assign is end;
architecture t of concurrent_assign is
  signal a : integer := 0;
  signal b, c, d : integer;
  signal sel : integer := 0;
begin
  b <= a + 1;
  c <= a * 2 when sel = 0 else a * 3 when sel = 1 else -1;
  with sel select d <= 100 when 0, 200 when 1, 300 when others;
  process (b, c, d) begin report "b=" & integer'image(b) & " c=" & integer'image(c) & " d=" & integer'image(d) & " @" & time'image(now); end process;
  process begin
    wait for 1 ns; a <= 5;
    wait for 1 ns; sel <= 1;
    wait for 1 ns; sel <= 2;
    wait;
  end process;
end;
