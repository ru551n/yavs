-- difftest: top=force_out_port std=2008
entity sub is port (o : out integer); end;
architecture a of sub is begin o <= 4; end;
entity force_out_port is end;
architecture t of force_out_port is
  signal s : integer := 0;
begin
  u : entity work.sub port map (o => s);
  process begin
    wait for 1 ns; report "s=" & integer'image(s);
    s <= force out 11; wait for 1 ns;
    report "force out s=" & integer'image(s);
    s <= release out; wait for 1 ns;
    report "release s=" & integer'image(s);
    wait;
  end process;
end;
