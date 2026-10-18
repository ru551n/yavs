-- yavs-test: std=2008
-- expect: E1100 5:26 `parsec` is not declared
-- LRM 5.2.4
package p is
  constant c : time := 5 parsec;
end package;
