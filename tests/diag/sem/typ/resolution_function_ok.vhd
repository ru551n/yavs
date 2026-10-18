-- yavs-test: std=2008
-- expect: none
-- LRM 4.6
package p is
  function res (a : bit_vector) return bit;
  subtype rbit is res bit;
end package;
