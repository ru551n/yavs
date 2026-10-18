-- difftest: top=postponed_basic std=2008
entity postponed_basic is end;
architecture t of postponed_basic is
  signal a, b : integer := 0;
begin
  process begin
    wait for 1 ns; a <= 1;
    wait for 1 ns; a <= 2;
    wait;
  end process;
  b <= a after 0 ns;
  postponed process (a, b) begin
    report "postponed a=" & integer'image(a) & " b=" & integer'image(b) & " @" & time'image(now);
  end process;
  process (a, b) begin
    report "normal a=" & integer'image(a) & " b=" & integer'image(b) & " @" & time'image(now);
  end process;
end;
