-- yavs-test: std=2008
-- expect: E1331 5:38 bit string 4X"1F" loses non-zero digits when truncated to 4 bits
-- LRM 15.8
package p is
  constant c : bit_vector(0 to 3) := 4X"1F";
end package;
