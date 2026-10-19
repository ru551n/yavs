-- yavs-test: std=2019
-- expect: none
-- LRM 2019 6.5.2, 6.5.4: anonymous parameter types of formal subprograms bind to the actual's profile
package p is
  generic (function cmp (a : type is private; b : a'subtype) return boolean);
  function k return integer;
end package;
package body p is
  function k return integer is begin return 1; end function;
end package body;
package q is
  function twice generic (function f (x : type is private) return integer) parameter (v : integer) return integer;
end package;
package body q is
  function twice generic (function f (x : type is private) return integer) parameter (v : integer) return integer is
  begin
    return v + v;
  end function;
end package body;
entity e is
  generic (function f (x : type is private) return integer);
end entity;
architecture a of e is begin end architecture;
entity top is end;
architecture a of top is
  function same (a, b : integer) return boolean is begin return a = b; end function;
  function same_t (a, b : time) return boolean is begin return a = b; end function;
  function seven (b : boolean) return integer is begin return 7; end function;
  function eight (r : real) return integer is begin return 8; end function;
  package ok1 is new work.p generic map (cmp => same);
  package ok2 is new work.p generic map (cmp => same_t);
  function t1 is new work.q.twice generic map (f => seven);
  function t2 is new work.q.twice generic map (f => eight);
begin
  u1 : entity work.e generic map (f => seven);
  u2 : entity work.e generic map (f => eight);
end architecture;
