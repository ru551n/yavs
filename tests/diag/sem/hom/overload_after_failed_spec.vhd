-- yavs-test: std=2008
-- expect: E1100 5:19 `missing_t` is not declared
-- LRM 4.5.1, sem.md 8.2 (the failed spec's poison name is not a homograph of the next overload)
package p is
  function f (x : missing_t) return integer;
  function f (x : integer) return integer;
end package;
