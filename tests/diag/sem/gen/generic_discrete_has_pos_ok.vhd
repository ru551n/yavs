-- yavs-test: std=2019
-- expect: none
-- LRM 6.5.3
package g is
  generic (type t is (<>));
  function p (a : t) return integer;
end package;
package body g is
  function p (a : t) return integer is
  begin
    return t'pos(a);
  end function;
end package body;
