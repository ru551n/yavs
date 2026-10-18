-- yavs-test: std=2008
-- expect: E1100 6:20 `ghost` is not declared
-- LRM 7.2
package p is
  attribute tag : integer;
  attribute tag of ghost : signal is 1;
end package;
