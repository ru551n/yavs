-- yavs-test: std=2008
-- expect: E1323 8:53 expected an element of type `BIT`, found a value of the aggregate type
-- LRM 9.3.3.3: an association with `others` gives an element value, not a piece
package p is
  constant a : bit_vector(3 downto 0) := "1100";
end package;
package body p is
  constant v : bit_vector(3 downto 0) := (others => a);
end package body;
