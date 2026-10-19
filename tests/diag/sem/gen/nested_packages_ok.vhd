-- yavs-test: std=2008
-- expect: none
-- LRM 4.7, 4.8, 12.1: a package declared in a package gets its body in the package body;
-- the nested body sees the outer body's declarations; local generic packages.
package outer is
  package inner is
    constant base : integer := 10;
    function scaled(x : integer) return integer;
  end package;
end package;
package body outer is
  constant offset : integer := 1;
  package body inner is
    function scaled(x : integer) return integer is
    begin
      return base * x + offset;
    end function;
  end package body;
  package g is
    generic (n : integer);
    function get return integer;
  end package;
  package g1 is new g generic map (n => 1);
  package body g is
    function get return integer is
    begin
      return n + inner.base;
    end function;
  end package body;
end package body;
