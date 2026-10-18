-- yavs-test: std=2008
-- expect: E1223 8:22 a generic cannot be of access type `int_ptr`
-- LRM 6.5.2
package p is
  type int_ptr is access integer;
end package;
use work.p.all;
entity e is generic (g : int_ptr := null);
end entity;
