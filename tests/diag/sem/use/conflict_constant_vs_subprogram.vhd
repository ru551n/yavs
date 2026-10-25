-- yavs-test: std=2008
-- expect: E1102 18:12 `flush` is ambiguous: several use clauses make it visible
-- LRM 12.4: a constant and a subprogram of one designator, both potentially visible by use
-- clauses, are not made directly visible (neither is a subprogram-only or literal-only set).
-- The implicit FLUSH of std.textio is not hidden by the explicit constant (#628).
package p is
  constant flush : integer := 1;
end package;

use work.p.all;
use std.textio.all;
package q is
  function f return integer;
end package;
package body q is
  function f return integer is
  begin
    return flush;
  end;
end package body;
