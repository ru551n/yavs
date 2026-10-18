-- yavs-test: std=2019
-- expect: E1100 9:18 view `nov` is not declared
-- LRM 2019 6.5.2
package p is
  type rec is record a : bit; end record;
end package;
use work.p.all;
entity e is
  port (x : view nov of rec);
end entity;
