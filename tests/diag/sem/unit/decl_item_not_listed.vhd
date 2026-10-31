-- yavs-test: std=2008
-- expect: E1239 8:3 a component declaration is not allowed in an entity
-- expect: E1239 11:3 a subprogram body is not allowed in a package declaration
-- expect: E1239 15:3 a package body is not allowed in a package declaration
-- expect: E1239 20:3 a signal declaration is not allowed in a package body
-- LRM 3.2.3, 4.7, 4.8 (#867)
entity e is
  component c is end component;
end entity;
package q is
  function f return integer is begin return 1; end function;
  package inner is
    constant k : integer := 3;
  end package;
  package body inner is
  end package body;
end package;
package r is end package;
package body r is
  signal s : bit;
end package body;
