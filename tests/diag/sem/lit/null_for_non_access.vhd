-- yavs-test: std=2008
-- expect: E1300 5:27 `null` requires an access type, found `integer`
-- LRM 9.3.2
package p is
  constant c : integer := null;
end package;
