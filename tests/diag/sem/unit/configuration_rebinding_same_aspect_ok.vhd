-- yavs-test: std=2008
-- expect: none
-- LRM 7.3.2.1: an incremental binding may repeat the primary binding's entity aspect
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
