-- yavs-test: std=2008
-- expect: none
-- LRM 6.9, 6.10, 7.2
package p is
  signal s : bit;
  constant c : integer := 1;
  type t is (a, b);
  function f return integer;
  group pair is (signal, constant);
  group any_sig is (signal <>);
  group names is (type, function, literal <>);
  group g1 : pair (s, c);
  group g2 : any_sig (s);
  group g3 : any_sig (s, s);
  group g4 : names (t, f, a, b);
  group g5 : names (t, f);
  attribute tag : integer;
  attribute tag of g1 : group is 3;
end package;
