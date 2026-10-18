-- difftest: top=tb_reject std=2008
entity tb_reject is end;
architecture t of tb_reject is
  signal s : integer := 0;
begin
  process begin
    s <= reject 2 ns inertial 1 after 10 ns;
    wait for 5 ns;
    s <= reject 2 ns inertial 2 after 4 ns;  -- matures at 9 ns, earlier than 10
    wait for 20 ns;
    s <= reject 3 ns inertial 7 after 5 ns;
    wait for 4 ns;
    s <= reject 3 ns inertial 8 after 5 ns;
    wait;
  end process;
  process (s) begin report "s=" & integer'image(s) & " @" & time'image(now); end process;
end;
