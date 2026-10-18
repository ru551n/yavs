-- yavs-test: std=2008
-- expect: E1300 18:27 not a function name
-- fuzz: report_name returned Reported for an object part without a diagnostic (validator ICE)
package g is
  type r is record f : integer; end record;
  constant c : r := (f => 1);
end;
use work.g.all;
entity e is
  generic (function f(x : integer) return integer);
end;
architecture t of e is begin end;
use work.g.all;
entity top is end;
architecture t of top is
  component e is generic (function f(x : integer) return integer); end component;
begin
  u : e generic map (f => c.f);
end;
