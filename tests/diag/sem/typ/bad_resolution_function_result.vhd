-- yavs-test: std=2008
-- expect: E1225 6:19 resolution function `res` must return `bit`
-- LRM 4.6
package p is
  function res (a : bit_vector) return integer;
  subtype rbit is res bit;
end package;
