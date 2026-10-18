-- yavs-test: std=2008
-- expect: E1325 5:30 `others` needs a constrained context subtype
-- LRM 9.3.3.3
package p is
  constant c : bit_vector := (others => '0');
end package;
