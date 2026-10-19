-- yavs-test: std=2008
-- expect: E1402 7:24 the integer literal is too large
-- expect: E1402 8:25 the physical literal is too large
-- LRM 9.3, 5.2.3: the range of universal_integer is implementation-defined; yavs rejects a literal beyond i64'high even when negated (user, 2026-10-19)
package p is
  type big is range -9223372036854775807 - 1 to 9223372036854775807;
  constant a : big := -9223372036854775808;
  constant c : time := -9223372036854775808 fs;
  constant d : big := -9223372036854775807 - 1 + 1;
end package;
