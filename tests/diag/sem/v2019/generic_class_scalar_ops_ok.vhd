-- yavs-test: std=2019
-- expect: none
-- LRM 2019 6.5.3
package g is
  generic (type t is (<>));
  function lt (a, b : t) return boolean;
end package;
package body g is
  function lt (a, b : t) return boolean is
  begin
    return a < b;
  end function;
end package body;
