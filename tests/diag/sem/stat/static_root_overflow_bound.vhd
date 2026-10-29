-- yavs-test: std=2008
-- expect: E1401 5:25 the bound 2 ** 32 - 1 is outside INTEGER
-- LRM 5.2.3.1
package p is
  type t is array (0 to 2 ** 32 - 1) of bit;
end package;
