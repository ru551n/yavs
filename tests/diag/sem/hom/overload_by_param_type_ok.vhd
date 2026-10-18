-- yavs-test: std=2008
-- expect: none
-- LRM 4.5.3
package p is
  function f (x : integer) return integer;
  function f (x : boolean) return integer;
  function f (x : integer; y : integer) return integer;
end package;
