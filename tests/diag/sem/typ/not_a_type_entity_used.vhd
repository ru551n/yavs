-- yavs-test: std=2008
-- expect: E1201 7:14 `e` is not a type or subtype
-- LRM 6.3
entity e is end entity;
use work.e;
package p is
  signal s : e;
end package;
