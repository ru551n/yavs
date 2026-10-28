-- yavs-test: std=2008
-- expect: E1360 5:12 function `f` has a parameter of class `variable`
-- LRM 4.2.2.1: function parameters are constants, signals or files (#808)
package p is
  function f (variable c : in integer) return integer;
end package;
