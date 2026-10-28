-- yavs-test: std=2008
-- expect: E1002 15:24 architecture `rtll` of entity `leaf` not found in library `work`
-- help: an architecture with a similar name exists: `rtl`
-- LRM 11.7
entity leaf is
  port (clk : in bit);
end entity;
architecture rtl of leaf is
begin
end architecture;
entity top is end entity;
architecture a of top is
  signal c : bit;
begin
  u : entity work.leaf(rtll) port map (clk => c);
end architecture;
