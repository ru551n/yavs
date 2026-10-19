-- yavs-test: std=2008
-- expect: E1702 16:7 instance `u1` is already bound by a configuration specification
-- LRM 3.4.2
entity sub is end entity;
architecture r of sub is begin end architecture;
entity e is end entity;
architecture a of e is
  component c is end component;
  for u1 : c use entity work.sub(r);
begin
  u1 : c;
end architecture;
configuration cfg of e is
  for a
    for u1 : c
      use entity work.sub(r);
    end for;
  end for;
end configuration;
