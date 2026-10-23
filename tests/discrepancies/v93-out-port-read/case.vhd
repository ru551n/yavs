entity leaf is port (o : out integer); end;
architecture a of leaf is
  signal s : integer := 0;
begin
  o <= 1;
  process begin wait for 1 ns; s <= o; wait; end process;
end;
entity top is end;
architecture a of top is
  signal v : integer;
begin
  u : entity work.leaf port map (o => v);
  process begin wait for 2 ns; report "RES " & integer'image(v); wait; end process;
end;
