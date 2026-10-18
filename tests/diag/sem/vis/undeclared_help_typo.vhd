-- rsim-test: std=2008
-- expect: E1100 7:33 `conter` is not declared
-- help: did you mean `counter`
-- LRM 12.3
package p is
  constant counter : integer := 1;
  constant other   : integer := conter;
end package;
