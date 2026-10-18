-- yavs-test: std=2008
-- expect: none
-- LRM 12.1, 4.3: an entity and its architecture are one declarative region, so a subprogram declared in the entity gets its body in the architecture (#177)
entity e is
  function f (y : integer) return integer;
end entity;
architecture a of e is
  function f (y : integer) return integer is
  begin
    return y + 1;
  end function;
begin
  process
  begin
    assert f(1) = 2;
    wait;
  end process;
end architecture;
