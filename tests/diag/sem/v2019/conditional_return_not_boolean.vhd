-- yavs-test: std=2019
-- expect: E1340 7:19 condition must be BOOLEAN, found `integer`
-- LRM 2019 10.14
package p is
  function f(x : integer) return integer is
  begin
    return 1 when x else 2;
  end function;
end package;
