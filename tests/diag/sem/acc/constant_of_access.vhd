-- yavs-test: std=2008
-- expect: E1223 6:12 a constant cannot be of access type `int_ptr`
-- LRM 6.4.2.2
package p is
  type int_ptr is access integer;
  constant c : int_ptr := new integer'(42);
end package;
