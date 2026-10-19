-- yavs-test: std=2008
-- expect: E1402 9:23 the literal without the minus is too large
-- expect: E1402 10:24 one past the minimum is too large
-- expect: E1402 11:24 physical literal without the minus is too large
-- expect: E1402 12:25 physical literal one past the minimum is too large
-- LRM 9.3, 5.2.3
package p is
  type big is range -9223372036854775807 - 1 to 9223372036854775807;
  constant a : big := 9223372036854775808;
  constant b : big := -9223372036854775809;
  constant c : time := 9223372036854775808 fs;
  constant d : time := -9223372036854775809 fs;
end package;
