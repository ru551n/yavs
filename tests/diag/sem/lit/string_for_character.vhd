-- yavs-test: std=2008
-- expect: E1300 5:29 expected `character`, found a string literal
-- LRM 15.7
package p is
  constant c : character := "a";
end package;
