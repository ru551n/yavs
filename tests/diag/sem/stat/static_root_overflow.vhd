-- yavs-test: std=2008
-- expect: W1401 5:27 the outermost value -2 ** 31 - 1 is outside INTEGER
-- LRM 5.2.3.1
package p is
  constant c : integer := -2 ** 31 - 1;
end package;
