-- difftest: top=tb_inertial std=2008
entity tb_inertial is end;
architecture t of tb_inertial is
  signal s : integer := 0;
begin
  process begin
    s <= 1 after 10 ns;
    wait for 5 ns;
    s <= 2 after 10 ns;
    wait for 20 ns;
    s <= 3 after 10 ns;
    wait for 5 ns;
    s <= 3 after 10 ns;
    wait;
  end process;
  process (s) begin report "s=" & integer'image(s) & " @" & time'image(now); end process;
end;
