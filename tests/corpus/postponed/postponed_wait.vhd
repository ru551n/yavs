-- difftest: top=postponed_wait std=2008
entity postponed_wait is end;
architecture t of postponed_wait is
  signal s : integer := 0;
begin
  process begin
    wait for 2 ns; s <= 1; wait for 0 ns; s <= 2; wait for 0 ns; s <= 3;
    wait for 5 ns; s <= 4;
    wait;
  end process;
  postponed process begin
    wait on s;
    report "settled s=" & integer'image(s) & " @" & time'image(now);
  end process;
end;
