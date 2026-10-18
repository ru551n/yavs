-- yavs-test: std=2008
-- expect: E1403 6:26 value 1.5 is outside subtype `unit_r`
-- LRM 4.3.2
package p is
  subtype unit_r is real range 0.0 to 1.0;
  constant c : unit_r := 1.5;
end package;
