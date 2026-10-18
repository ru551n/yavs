-- yavs-test: std=2008 flags=--relaxed
-- expect: W1904 6:19 shared variable `sv` is not of a protected type
-- help: --relaxed accepts a shared variable of a non-protected type
-- LRM 6.4.2.4
package p is
  shared variable sv : integer;
end package;
