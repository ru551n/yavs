-- yavs-test: std=2008
-- expect: E1226 10:24 deferred constant `S` is used before its full declaration
-- expect: E1226 11:27 deferred constant `B` is used before its full declaration
-- LRM 6.4.2.2: before the end of its full declaration a deferred constant is named only in a default expression of a local generic, port or formal parameter (#809)
package p is
  constant C, S : real;
  constant A, B : integer;
end package;
package body p is
  constant S : real := S;
  constant A : integer := B;
  constant B : integer := 1;
  constant C : real := real(B);
  function f (x : real := C) return real is begin return x + S; end;
end package body;
