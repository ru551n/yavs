-- yavs-test: std=2008 flags=--design-std=2019
-- expect: none
-- LRM 2019 0.1
package p is
  constant a : integer := 100;
  constant b : integer := a * 2;
end package;
