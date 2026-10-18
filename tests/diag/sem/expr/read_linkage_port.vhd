-- yavs-test: std=2008
-- expect: E1360 10:8 cannot read port `l` of mode `linkage`
-- LRM 6.5.6.3
entity e is
  port (l : linkage bit);
end entity;
architecture a of e is
  signal s : bit;
begin
  s <= l;
end architecture;
