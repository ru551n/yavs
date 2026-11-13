-- yavs-test: std=2008
-- expect: E1230 5:13 procedure `bump` has no body
-- LRM 4.2.1, 4.3: the package body has no body for a header-instance procedure spec (#1136)
package p is
  procedure bump generic (constant n : integer) generic map (n => 7)
    parameter (variable x : inout integer);
end package;
package body p is
end package body;
