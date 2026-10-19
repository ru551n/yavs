-- yavs-test: std=2008
-- expect: E1101 5:30 parameter `a` is declared twice
-- LRM 6.5.2
package p is
  procedure pr (a : integer; a : integer);
end package;
