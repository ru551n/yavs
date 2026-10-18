-- yavs-test: std=2008
-- expect: E1300 8:17 not a function name
-- fuzz: report_name returned Reported for an object part without a diagnostic (validator ICE)
entity e is end;
architecture t of e is
  type r is record f : bit; end record;
  signal s : r;
  subtype st is s.f bit;
begin
end;
