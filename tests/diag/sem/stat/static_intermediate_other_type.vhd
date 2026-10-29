-- yavs-test: std=2008
-- expect: W1401 5:24 2 ** 40 is an INTEGER operand of the TIME multiplication
-- LRM 5.2.3.1
package p is
  constant t : time := 2 ** 40 * 1 fs;
end package;
