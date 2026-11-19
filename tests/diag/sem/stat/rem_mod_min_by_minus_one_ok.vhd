-- yavs-test: std=2008
-- expect: none
-- LRM 9.2.7
package p is
  type long is range -9223372036854775807 - 1 to 9223372036854775807;
  constant a : long := long'low rem (-1);
  constant b : long := long'low mod (-1);
  constant c : integer := integer'low rem (-1);
  constant d : integer := integer'low mod (-1);
  constant e : long := long'low mod 3;
  constant f : long := long'low rem (-3);
  constant g : integer := integer'low mod (-7) + integer'low rem 7;
end package;
