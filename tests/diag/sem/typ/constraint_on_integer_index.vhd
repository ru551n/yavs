-- yavs-test: std=2008
-- expect: E1211 5:21 an index constraint cannot be applied to scalar type `integer`
-- LRM 6.3
package p is
  signal s : integer(7 downto 0);
end package;
