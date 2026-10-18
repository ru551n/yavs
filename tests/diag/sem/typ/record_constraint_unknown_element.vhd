-- yavs-test: std=2008
-- expect: E1213 6:16 record `r` has no element `zz`
-- LRM 5.3.3
package p is
  type r is record a : bit_vector; b : integer; end record;
  signal s : r(zz(3 downto 0));
end package;
