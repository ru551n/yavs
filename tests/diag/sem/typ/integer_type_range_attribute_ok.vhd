-- yavs-test: std=2008
-- expect: none
-- LRM 3.1.2 (the range of an integer type definition is a range attribute; #646)
package p is
  type t2 is array (1 to 2, 3 to 9) of integer;
  type t3 is array (t2'range(1), t2'reverse_range(2)) of integer;
  type r1 is range t2'range(1);
  type r2 is range t3'range(2);
  constant ok : boolean := r1'left = 1 and r1'right = 2 and r2'left = 9 and r2'right = 3;
end package;
