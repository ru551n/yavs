-- yavs-test: std=2008
-- expect: E1300 8:26 `s.f` is not a named entity
-- fuzz: report_name returned Reported for an object part without a diagnostic (validator ICE)
entity e is end;
architecture t of e is
  type r is record f : bit; end record;
  signal s : r;
  constant n : string := s.f'simple_name;
begin
end;
