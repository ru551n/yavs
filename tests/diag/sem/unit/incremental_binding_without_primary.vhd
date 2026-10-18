-- yavs-test: std=2008
-- expect: E1704 14:15 this binding adds maps, but no configuration specification binds the instance to an entity
-- LRM 7.3.2.1
entity c is generic (n : integer := 0); end entity;
architecture rtl of c is begin end architecture;
entity top is end entity;
architecture a of top is
  component c is generic (n : integer := 0); end component;
begin
  u : c;
end architecture;
configuration cfg of top is
  for a
    for u : c generic map (n => 2); end for;
  end for;
end configuration;
