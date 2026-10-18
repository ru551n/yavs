-- yavs-test: std=2008
-- expect: E1105 11:12 `hidden` is not in the visible part of `p`
-- LRM 12.4
package p is
  function f return integer;
end package;
package body p is
  constant hidden : integer := 4;
  function f return integer is begin return hidden; end function;
end package body;
use work.p.hidden;
package q is
end package;
