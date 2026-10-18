-- yavs-test: std=2008
-- expect: E1005 7:11 `work` is not allowed in a library clause of a context declaration
-- expect: E1005 8:7 `work` is not allowed as a prefix in a context declaration
-- LRM 13.4
package p is constant c : integer := 1; end package;
context ctx is
  library work;
  use work.p.all;
end context;
