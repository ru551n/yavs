-- yavs-test: std=2008
-- expect: E1300 7:21 not a subprogram name
-- fuzz: report_name returned Reported for an object part without a diagnostic (validator ICE)
package g is
  type r is record f : integer; end record;
  constant c : r := (f => 1);
  function f is new c.f generic map (x => 1);
end;
