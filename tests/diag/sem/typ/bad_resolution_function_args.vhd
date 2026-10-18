-- yavs-test: std=2008
-- expect: E1225 6:19 resolution function `res` must have one parameter of an array type
-- LRM 4.6
package p is
  function res (a, b : bit) return bit;
  subtype rbit is res bit;
end package;
