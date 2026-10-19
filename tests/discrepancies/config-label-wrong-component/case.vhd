-- yavs-test: std=2008
-- expect: E1701 13:9 instance `u1` is of component `c1`, not `c2`
-- LRM 3.4.2
entity e is end entity;
architecture a of e is
  component c1 is end component;
  component c2 is end component;
begin
  u1 : c1;
end architecture;
configuration cfg of e is
  for a
    for u1 : c2
    end for;
  end for;
end configuration;
