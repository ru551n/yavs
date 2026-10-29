-- yavs-test: std=2008
-- expect: E1999 10:14 a package body of a package with a generic map is not implemented
-- LRM 4.7, 4.8: not implemented yet (no reference simulator handles the body of such a package)
package gm is
  generic (g : integer := 1);
  generic map (g => 2);
  function f return integer;
end package;

package body gm is
  function f return integer is begin return g; end function;
end package body;
