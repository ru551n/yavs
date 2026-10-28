-- yavs-test: std=2008
-- expect: E1226 11:42 deferred constant `S` is used before its full declaration
-- expect: E1226 13:26 deferred constant `S` is used before its full declaration
-- LRM 6.4.2.2: a subprogram body before the full declaration may not name the deferred constant; a parameter default may, and so may a body after it (#846)
package p is
  constant S : real;
  function f return real;
  function h (x : real := S) return real;
end package;
package body p is
  function f return real is begin return S; end;
  function g return real is
    variable v : real := S;
  begin
    return v;
  end;
  function h (x : real := S) return real is begin return x; end;
  constant S : real := 1.0;
  function k return real is begin return S; end;
end package body;
