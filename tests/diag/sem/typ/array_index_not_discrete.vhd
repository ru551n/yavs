-- yavs-test: std=2008
-- expect: E1201 5:22 index of an array must be a discrete type, found `real`
-- LRM 5.3.2
package p is
  type arr is array (real range <>) of bit;
end package;
