-- yavs-test: std=2008
-- expect: E1239 8:5 a component declaration is not allowed in a subprogram
-- LRM 4.3: a component declaration is not a subprogram declarative item (#808)
entity e is
end entity;
architecture a of e is
  function m return boolean is
    component c port (i : out real); end component;
  begin
    return false;
  end function;
begin
end architecture;
