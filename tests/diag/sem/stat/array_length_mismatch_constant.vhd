-- yavs-test: std=2008
-- expect: E1405 5:38 length mismatch: target has 4 elements, value has 3
-- LRM 4.3.2
package p is
  constant c : bit_vector(0 to 3) := "101";
end package;
