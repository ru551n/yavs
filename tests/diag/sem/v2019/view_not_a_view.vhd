-- yavs-test: std=2019
-- expect: E1902 10:18 `k` is not a mode view
-- LRM 2019 6.5.2
package p is
  type rec is record a : bit; end record;
  constant k : integer := 1;
end package;
use work.p.all;
entity e is
  port (x : view k of rec);
end entity;
