-- difftest: top=signal_update_order std=2008
entity signal_update_order is end;
architecture t of signal_update_order is
  signal a, b : integer := 1;
begin
  process begin
    wait for 1 ns;
    a <= b; b <= a;
    wait for 0 ns;
    report "swap a=" & integer'image(a) & " b=" & integer'image(b);
    a <= 10; a <= 11;
    wait for 0 ns;
    report "last wins a=" & integer'image(a);
    wait;
  end process;
end;
