-- yavs-test: std=2008
-- expect: E1230 6:12 no body in the entity or the architecture
-- LRM 4.3: the body of a subprogram declared in an entity belongs in the same declarative region (entity plus architecture)
entity e is
  procedure x (y : integer);
  function f (y : integer) return integer;
end entity;
architecture a of e is
  procedure x (y : integer) is
  begin
  end procedure;
begin
end architecture;
