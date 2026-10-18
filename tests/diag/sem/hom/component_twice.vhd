-- yavs-test: std=2008
-- expect: E1101 6:13 component `c` is already declared in this region
-- LRM 12.3
package p is
  component c is end component;
  component c is end component;
end package;
