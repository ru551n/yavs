-- yavs-test: std=2008
-- expect: E1300 9:17 expected `time`, found `integer`
-- LRM 16.2
entity e is end entity;
architecture a of e is
  signal s : bit;
  signal q : boolean;
begin
  q <= s'stable(1);
end architecture;
