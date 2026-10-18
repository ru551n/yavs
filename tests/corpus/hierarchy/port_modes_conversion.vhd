-- difftest: top=port_modes_conversion std=2008
entity sub is port (i : in integer; o : out integer; io : inout integer; b : buffer integer); end;
architecture a of sub is begin
  o <= i + 1;
  io <= i + 10;
  b <= i * 2;
end;
entity port_modes_conversion is end;
architecture t of port_modes_conversion is
  signal i, o, io, b : integer := 0;
  signal dummy : integer;
begin
  u : entity work.sub port map (i, o, io, b);
  process begin
    for k in 1 to 3 loop
      i <= k; wait for 1 ns;
      report "i=" & integer'image(i) & " o=" & integer'image(o) & " b=" & integer'image(b);
    end loop;
    wait;
  end process;
end;
