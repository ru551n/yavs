-- yavs-test: std=2019
-- expect: E1340 7:17 condition must be BOOLEAN, found `integer`
-- LRM 2019 10.14
package p is
  procedure q(x : integer) is
  begin
    return when x;
  end procedure;
end package;
