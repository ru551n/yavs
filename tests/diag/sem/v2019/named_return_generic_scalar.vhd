-- yavs-test: std=2019
-- expect: E1233 6:41 a named return value needs an array or record type, not `INTEGER`
-- LRM 2019 4.2.1 (lrm-checks L77): a generic type is checked in each instance, reported at
-- the return identifier
package p is
  function zero generic (type t) return r of t;
end package;
package body p is
  function zero generic (type t) return r of t is
    variable o : r;
  begin
    return o;
  end function;
  function zi is new zero generic map (t => integer);
end package body;
