-- yavs-test: std=2008
-- expect: E1501 7:23 `p` is not a generic package
-- LRM 4.9
package p is
  constant c : integer := 1;
end package;
package i is new work.p generic map (c => 1);
