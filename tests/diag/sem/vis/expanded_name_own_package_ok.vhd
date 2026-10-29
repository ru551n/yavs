-- yavs-test: std=2008
-- expect: none
-- LRM 8.3, 12.1: inside package `p` or its body, `p.x` names `x` of the declaration or the
-- body (one region), also overloads, a generic package and a nested package (#854)
package p is
  constant S : real := 1.0;
  function k return real;
  function f(x : integer) return integer;
  function f(x : real) return real;
  constant T : real := p.S + 1.0;
end package;
package body p is
  constant B : integer := 5;
  function f(x : integer) return integer is begin return x + p.B; end;
  function f(x : real) return real is begin return x; end;
  function k return real is
    constant S : real := 9.0;
  begin
    return p.S + p.T + p.f(1.5) + real(p.f(1)) + S;
  end;
end package body;

package gp is
  generic (n : integer);
  constant S : integer := n;
  function k return integer;
end package;
package body gp is
  function k return integer is begin return gp.S; end;
end package body;

package q is
  function g return integer;
end package;
package body q is
  package inner is
    constant C : integer := 3;
    function g return integer;
  end package;
  package body inner is
    constant D : integer := 4;
    function g return integer is begin return inner.C + inner.D; end;
  end package body;
  function g return integer is begin return inner.g; end;
end package body;

