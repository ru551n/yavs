-- yavs-test: std=2008
-- expect: E1702 19:7 these instances is already bound by a configuration specification
-- LRM 3.4.3, 7.3.1: `others` includes u2, which a configuration specification binds
entity sub is end entity;
architecture r of sub is begin end architecture;
entity sub2 is end entity;
architecture r of sub2 is begin end architecture;
entity e is end entity;
architecture a of e is
  component c is end component;
  for u2 : c use entity work.sub(r);
begin
  u1 : c;
  u2 : c;
end architecture;
configuration cfg of e is
  for a
    for others : c
      use entity work.sub2(r);
    end for;
  end for;
end configuration;
