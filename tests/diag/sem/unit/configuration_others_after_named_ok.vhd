-- yavs-test: std=2008
-- expect: none
-- LRM 3.4.3, 7.3.1: `others` leaves out u2, which a configuration specification binds
entity sub is end entity;
architecture r of sub is begin end architecture;
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
    for u2 : c
    end for;
    for others : c
      use entity work.sub(r);
    end for;
  end for;
end configuration;
