-- yavs-test: std=2008
-- expect: E1301 5:27 no operator `+` matches the operands
-- LRM 9.2
package p is
  constant c : boolean := true + 1;
end package;
