-- LRM 8.7: a relative external pathname used as a port actual starts at the region that contains
-- the port map (here top), not at the instantiated entity (mon, which has no `l`).
entity leaf is end entity;
architecture a of leaf is
  signal hidden : natural := 5;
begin
end architecture;
entity mon is port (i : in natural); end entity;
architecture a of mon is
begin
  process begin
    wait for 1 ns;
    report "RES rel i=" & integer'image(i);
    wait;
  end process;
end architecture;
entity top is end entity;
architecture a of top is
begin
  l : entity work.leaf;
  m : entity work.mon port map (i => << signal l.hidden : natural >>);
end architecture;
