-- rsim-test: std=2008
-- expect: E1703 9:38 component port `b` has no matching port in entity `sub`
-- LRM 7.3.2
entity sub is port (a : in bit); end entity;
architecture r of sub is begin end architecture;
entity e is end entity;
architecture a of e is
  component c is port (b : in bit); end component;
  for all : c use entity work.sub(r) port map (b => b);
begin
end architecture;
