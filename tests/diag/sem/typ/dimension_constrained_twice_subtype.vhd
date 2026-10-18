-- yavs-test: std=2008
-- expect: E1210 6:17 index range of `v8` is already constrained
-- LRM 6.3
package p is
  subtype v8 is bit_vector(7 downto 0);
  signal s : v8(3 downto 0);
end package;
