-- yavs-test: std=2008
-- expect: E1100 5:14 type `word` is not declared
-- LRM 12.3
package p is
  signal s : word;
end package;
