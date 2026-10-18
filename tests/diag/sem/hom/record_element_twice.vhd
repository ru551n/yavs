-- yavs-test: std=2008
-- expect: E1101 7:5 element `a` is declared twice
-- LRM 5.3.3
package p is
  type r is record
    a : integer;
    a : bit;
  end record;
end package;
