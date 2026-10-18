-- yavs-test: std=2008
-- expect: E1403 5:27 value -1 is outside subtype `natural`
-- LRM 4.3.2
package p is
  constant c : natural := -1;
end package;
