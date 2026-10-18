-- yavs-test: std=2008
-- expect: E1214 6:15 type `m` has 2 index ranges, the constraint has 1
-- LRM 5.3.2.2
package p is
  type m is array (natural range <>, natural range <>) of bit;
  signal s : m(0 to 3);
end package;
