-- yavs-test: std=2008
-- expect: E1641 8:22 signal where a constant is wanted
-- LRM 6.10
package p is
  signal s : bit;
  constant c : integer := 1;
  group pair is (signal, constant);
  group g : pair (s, s);
end package;
