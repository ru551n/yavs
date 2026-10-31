-- yavs-test: std=2019
-- expect: E1340 9:17 condition must be BOOLEAN, found `integer`
-- LRM 2019 10.14
package p is
end package;
package body p is
  procedure q(x : integer) is
  begin
    return when x;
  end procedure;
end package body;
