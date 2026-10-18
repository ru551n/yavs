-- yavs-test: std=2019
-- expect: none
-- LRM 2019 16.2
package p is
  type r is record a : integer; end record;
  constant x : r := (a => 1);
  constant s : string := r'image(x);
end package;
