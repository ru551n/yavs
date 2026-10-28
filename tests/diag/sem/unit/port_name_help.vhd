-- yavs-test: std=2008
-- expect: E1100 15:34 entity `leaf` has no port `clck`
-- help: a port with a similar name exists: `clk`
-- LRM 6.5.7.3
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
  u : entity work.leaf port map (clck => c);
end architecture;
