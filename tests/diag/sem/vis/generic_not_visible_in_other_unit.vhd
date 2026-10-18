-- yavs-test: std=2008
-- expect: E1100 13:27 `g` is not declared
-- LRM 12.2
entity e1 is
  generic (g : integer := 1);
end entity;
architecture a of e1 is
begin
end architecture;
entity e2 is
end entity;
architecture a of e2 is
  constant c : integer := g;
begin
end architecture;
