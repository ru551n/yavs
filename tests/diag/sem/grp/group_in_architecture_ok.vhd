-- yavs-test: std=2008
-- expect: none
-- LRM 6.10
entity e is
  port (p : in bit);
end entity;
architecture a of e is
  signal s : bit;
  function f return integer is begin return 1; end function;
  group ents is (entity, architecture, signal, function);
  group g : ents (e, a, s, f);
  group ports is (signal <>);
  group h : ports (p, s);
  attribute tag : integer;
  attribute tag of g, h : group is 1;
begin
end architecture;
