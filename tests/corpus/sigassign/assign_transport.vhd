-- difftest: top=tb_transport std=2008
entity tb_transport is end;
architecture t of tb_transport is
  signal s : integer := 0;
begin
  process begin
    s <= transport 1 after 10 ns;
    s <= transport 2 after 5 ns;
    wait;
  end process;
  process (s) begin report "s=" & integer'image(s) & " @" & time'image(now); end process;
end;
