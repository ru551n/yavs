-- yavs-test: std=2008
-- expect: E1360 5:12 function parameters must have mode `in`
-- LRM 4.2.2
package p is
  function f (x : out integer) return integer;
end package;
