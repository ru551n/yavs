-- yavs-test: std=2008
-- expect: none
-- LRM 16.2
package p is
  constant c : string := integer'simple_name;
end package;
