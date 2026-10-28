-- yavs-test: std=2008
-- expect: E1100 7:31 `nothing_like_it` is not declared
-- LRM 12.3
package p is
  constant width : integer := 4;
  signal count : integer;
  constant other : integer := nothing_like_it;
end package;
