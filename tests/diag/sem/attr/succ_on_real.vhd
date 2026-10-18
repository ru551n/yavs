-- yavs-test: std=2008
-- expect: E1621 5:29 attribute `succ` is not defined for floating-point type `real`
-- LRM 16.2
package p is
  constant c : real := real'succ(1.0);
end package;
