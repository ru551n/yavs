-- yavs-test: std=2008
-- expect: E1701 14:9 `u9` is not a component instance in `a`
-- LRM 3.4.2
entity sub is end entity;
architecture r of sub is begin end architecture;
entity e is end entity;
architecture a of e is
  component c is end component;
begin
  u1 : entity work.sub;
end architecture;
configuration cfg of e is
  for a
    for u9 : c
    end for;
  end for;
end configuration;
