-- yavs-test: std=2008
-- expect: E1640 5:3 `<>` only on the last entry
-- LRM 6.9
package p is
  group bad is (signal <>, constant);
end package;
