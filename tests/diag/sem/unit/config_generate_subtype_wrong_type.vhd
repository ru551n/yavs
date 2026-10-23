-- yavs-test: std=2008
-- expect: E1300 12:11 expected a range of `INTEGER`, found one of `CHARACTER`
-- LRM 3.4.2: the discrete range of a block configuration has the type of the generate parameter (#35)
entity e is end entity;
architecture a of e is
begin
  g : for i in 1 to 3 generate
  end generate;
end architecture;
configuration c of e is
  for a
    for g(character range 'a' to 'b')
    end for;
  end for;
end configuration;
