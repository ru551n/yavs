-- yavs-test: std=2008
-- expect: none
-- LRM 15.8
package p is
  constant c : bit_vector(0 to 7) := X"A5";
  constant d : bit_vector(0 to 4) := 5B"10101";
  constant e : bit_vector(0 to 7) := 8SX"F";
end package;
