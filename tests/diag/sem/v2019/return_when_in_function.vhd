-- yavs-test: std=2019
-- expect: E1802 10:5 a function return needs a value
-- LRM 2019 10.14: a function returns a value
package p is
  function f(x : integer) return integer;
end package;
package body p is
  function f(x : integer) return integer is
  begin
    return when x < 2;
    return 3;
  end function;
end package body;
