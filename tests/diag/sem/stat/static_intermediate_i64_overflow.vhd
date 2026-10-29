-- yavs-test: std=2019
-- expect: W1401 5:27 2 ** 63 overflows the 64-bit arithmetic
-- LRM 5.2.3.1
package p is
  constant c : integer := 2 ** 63 - 1;
end package;
