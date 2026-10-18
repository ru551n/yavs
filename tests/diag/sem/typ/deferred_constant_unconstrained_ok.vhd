-- yavs-test: std=2008
-- expect: none
-- LRM 6.4.2.2
package p is
  constant c : bit_vector;
end package;
package body p is
  constant c : bit_vector := "101";
end package body;
