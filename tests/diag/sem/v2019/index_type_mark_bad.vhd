-- yavs-test: std=2019
-- expect: E1623 7:14 no third dimension
-- expect: E1201 8:14 not an array
-- LRM 16.2 (2019): A'INDEX(n) needs an array prefix with an n-th dimension
package p is
  type big_t is array (natural range <>, character range <>) of integer;
  signal j : big_t'index(3);
  signal k : integer'index;
end package;
