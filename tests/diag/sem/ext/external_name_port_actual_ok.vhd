-- yavs-test: std=2008
-- expect: none
-- LRM 8.7 (an external name as a port map actual; sim1 accepts)
entity leaf is
  port (o : out bit);
end entity;
architecture a of leaf is
  signal hidden : bit := '1';
begin
  o <= hidden;
end architecture;
entity mon is
  port (i : in bit);
end entity;
architecture a of mon is
begin
  process (i) begin report "i=" & bit'image(i); end process;
end architecture;
entity top is end entity;
architecture a of top is
  signal x : bit;
begin
  l : entity work.leaf port map (o => x);
  m : entity work.mon port map (i => << signal .top.l.hidden : bit >>);
end architecture;
