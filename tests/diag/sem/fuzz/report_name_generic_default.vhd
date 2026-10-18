-- yavs-test: std=2008
-- expect: E1300 10:54 not a subprogram name
-- fuzz: report_name returned Reported for an object part without a diagnostic (validator ICE)
package g is
  type r is record f : integer; end record;
  constant c : r := (f => 1);
end;
use work.g.all;
entity e is
  generic (function f(x : integer) return integer is c.f);
end;
