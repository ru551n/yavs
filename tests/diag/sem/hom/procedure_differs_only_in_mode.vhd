-- yavs-test: std=2008
-- expect: E1101 6:13 `pr` with the same profile is already declared
-- LRM 4.5.3
package p is
  procedure pr (x : in integer);
  procedure pr (x : out integer);
end package;
