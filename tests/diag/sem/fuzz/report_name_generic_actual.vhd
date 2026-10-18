-- yavs-test: std=2008
-- expect: E1300 11:45 not a subprogram name
-- fuzz: report_name returned Reported for an object part without a diagnostic (validator ICE)
package g is
  generic (function f(x : integer) return integer);
end;
package h is
  type r is record f : integer; end record;
  constant c : r := (f => 1);
  function q(x : integer) return integer;
  package i is new work.g generic map (f => c.f);
end;
