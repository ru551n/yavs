-- yavs-test: std=2008
-- expect: none
-- LRM 9.3.6
package p is
  constant c : integer := integer(2.5);
  constant d : real := real(3);
end package;
