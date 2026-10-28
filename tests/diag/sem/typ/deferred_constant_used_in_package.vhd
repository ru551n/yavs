-- yavs-test: std=2008
-- expect: E1226 7:27 deferred constant `C` is used before its full declaration
-- LRM 6.4.2.2: a deferred constant has no value in its package declaration (#809)
package p is
  constant C : integer;
  function f (x : integer := C) return integer;
  constant D : integer := C + 1;
end package;
