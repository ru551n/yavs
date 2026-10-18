-- yavs-test: std=2008
-- expect: E1100 9:48 entity `sub` has no port `b`
-- LRM 7.3.2.3
entity sub is port (a : in bit := '0'); end entity;
architecture r of sub is begin end architecture;
entity e is end entity;
architecture a of e is
  component c is port (b : in bit); end component;
  for all : c use entity work.sub(r) port map (b => b);
begin
end architecture;
