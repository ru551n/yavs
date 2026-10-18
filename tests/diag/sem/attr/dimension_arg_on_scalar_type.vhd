-- yavs-test: std=2008
-- expect: E1007 5:35 `'length` of a scalar type mark needs VHDL-2019
-- LRM 16.2
package p is
  constant c : integer := integer'length;
end package;
