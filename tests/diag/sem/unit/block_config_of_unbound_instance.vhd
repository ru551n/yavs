-- yavs-test: std=2008
-- expect: E1704 13:7 no entity is bound to component `c`, so it has no architecture to configure
-- LRM 3.4.3
entity top is end entity;
architecture a of top is
  component c is generic (n : integer := 0); end component;
begin
  u : c;
end architecture;
configuration cfg of top is
  for a
    for u : c
      for rtl end for;
    end for;
  end for;
end configuration;
