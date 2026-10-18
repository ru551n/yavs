-- rsim-test: std=2008
-- expect: E1621 5:35 attribute `length` is not defined for scalar type `integer`
-- LRM 16.2
package p is
  constant c : integer := integer'length;
end package;
