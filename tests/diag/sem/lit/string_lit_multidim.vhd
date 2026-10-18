-- yavs-test: std=2008
-- expect: E1300 6:21 a string literal needs a 1-dimensional array type
-- LRM 15.7
package p is
  type m is array (0 to 1, 0 to 1) of character;
  constant c : m := "abcd";
end package;
