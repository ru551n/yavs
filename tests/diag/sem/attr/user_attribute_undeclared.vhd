-- yavs-test: std=2008
-- expect: E1100 6:13 attribute `tag` is not declared
-- LRM 7.2
package p is
  signal s : bit;
  attribute tag of s : signal is 1;
end package;
