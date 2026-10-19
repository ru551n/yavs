-- yavs-test: std=2019
-- expect: none
-- LRM 2019 4.2.1 (lrm-checks L77): a named return value of a generic type whose actual is a
-- scalar type, and of a generic scalar type class
package p is
  function zero generic (type t) return r of t;
  function low generic (type t is (<>)) return r of t;
end package;
package body p is
  function zero generic (type t) return r of t is
    variable o : r;
  begin
    return o;
  end function;
  function low generic (type t is (<>)) return r of t is
  begin
    return r'low;
  end function;
  function zi is new zero generic map (t => integer);
  function lb is new low generic map (t => boolean);
end package body;
