-- yavs-test: std=2019
-- expect: E1236 6:16 a subprogram parameter cannot have mode `linkage`
-- help: use `in`, `out` or `inout`
-- LRM 4.2.2.1: subprogram parameters are of mode in, out or inout
package p is
  procedure q (signal x : linkage bit);
end package;
