-- yavs-test: std=2008
-- expect: E1100 6:14 `e` is not declared
-- LRM 12.3, 12.4: library units of `work` are not directly visible without a use clause
entity e is end entity;
package p is
  signal s : e;
end package;
