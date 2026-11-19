-- yavs-test: std=2008
-- expect: W1401 6:24 long'low / (-1) overflows
-- LRM 9.2.7
package p is
  type long is range -9223372036854775807 - 1 to 9223372036854775807;
  constant a : long := long'low / (-1);
end package;
