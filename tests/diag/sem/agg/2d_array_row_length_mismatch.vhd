-- yavs-test: std=2008
-- expect: E1405 6:34 row has 3 elements, expected 2
-- LRM 9.3.3.3
package p is
  type m is array (0 to 1, 0 to 1) of bit;
  constant c : m := (('0', '1'), ('1', '0', '1'));
end package;
