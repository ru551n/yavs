-- yavs-test: std=2008
-- expect: E1213 6:31 element `a` is constrained twice
-- LRM 5.3.3
package p is
  type r is record a : bit_vector; b : integer; end record;
  signal s : r(a(3 downto 0), a(1 downto 0));
end package;
