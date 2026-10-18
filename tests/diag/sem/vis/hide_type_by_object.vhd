-- yavs-test: std=2008
-- expect: E1201 6:14 `bit` denotes a signal, not a type
-- LRM 12.3
package p is
  signal bit : integer;
  signal s : bit;
end package;
