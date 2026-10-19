-- yavs-test: std=2008
-- expect: E1236 6:16 a subprogram parameter cannot have mode `buffer`
-- help: use `in`, `out` or `inout`
-- LRM 4.2.2.1: subprogram parameters are of mode in, out or inout
package p is
  procedure q (signal x : buffer bit);
end package;
