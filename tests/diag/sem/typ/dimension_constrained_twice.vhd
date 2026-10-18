-- yavs-test: std=2008
-- expect: E1210 6:20 index range of `arr` is already constrained
-- LRM 6.3
package p is
  type arr is array (0 to 7) of bit;
  subtype s is arr(0 to 3);
end package;
