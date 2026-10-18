-- yavs-test: std=2008
-- expect: E1703 9:26 component port `b` has no matching port in entity `sub`
-- LRM 7.3.3 (default port map of a binding indication)
entity sub is port (a : in bit := '0'); end entity;
architecture r of sub is begin end architecture;
entity e is end entity;
architecture a of e is
  component c is port (b : in bit); end component;
  for all : c use entity work.sub(r);
begin
end architecture;
