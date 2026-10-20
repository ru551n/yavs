-- yavs-test: std=2008
-- expect: E1403 7:30 static value is outside subtype `bitrange`
-- LRM 9.3.5: the operand of a qualified expression belongs to the subtype of the type mark
package p is
  subtype bitrange is natural range 0 to 1;
  constant ok : integer := bitrange'(1);
  constant c : integer := bitrange'(5);
end package;
