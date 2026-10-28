-- yavs-test: std=2008
-- expect: E1002 15:19 design unit `leff` not found in library `work`
-- help: a design unit with a similar name exists: `leaf`
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
  u : entity work.leff port map (clk => c);
end architecture;
