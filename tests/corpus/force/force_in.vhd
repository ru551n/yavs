-- difftest: top=force_in std=2008
entity force_in is end;
architecture t of force_in is
  signal s : integer := 0;
begin
  process begin
    s <= 1; wait for 1 ns;
    report "s=" & integer'image(s);
    s <= force 5; wait for 1 ns;
    report "forced s=" & integer'image(s);
    s <= 7; wait for 1 ns;
    report "assign while forced s=" & integer'image(s);
    s <= release; wait for 1 ns;
    report "released s=" & integer'image(s);
    s <= 9; wait for 1 ns;
    report "assigned s=" & integer'image(s);
    wait;
  end process;
end;
