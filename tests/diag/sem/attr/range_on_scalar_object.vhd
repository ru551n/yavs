-- yavs-test: std=2008
-- expect: E1621 8:27 attribute `range` is not defined for scalar `a`
-- LRM 16.2
package p is
  constant a : integer := 1;
  type t is array (0 to 3) of bit;
  signal s : t;
  signal r : bit_vector(a'range);
end package;
