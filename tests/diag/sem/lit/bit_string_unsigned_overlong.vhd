-- yavs-test: std=2008
-- expect: E1331 5:38 bit string 2B"101" loses non-zero digits when truncated to 2 bits
-- LRM 15.8
package p is
  constant c : bit_vector(0 to 1) := 2B"101";
end package;
