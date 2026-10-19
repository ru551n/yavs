-- yavs-test: std=2019
-- expect: none
-- LRM 2019 4.2.1, 6.5.2: a generic subprogram with anonymous parameter types, instantiated and called
package p is
  function pick generic (type t) parameter (x : type is private; y : t) return t;
  function sum generic (type t) parameter (a : type is range <>; b : a'subtype; c : t) return a'subtype;
end package;
package body p is
  function pick generic (type t) parameter (x : type is private; y : t) return t is
  begin
    return y;
  end function;
  function sum generic (type t) parameter (a : type is range <>; b : a'subtype; c : t) return a'subtype is
  begin
    return a + b;
  end function;
end package body;
entity top is end;
architecture a of top is
  function pi is new work.p.pick generic map (t => integer);
  function pr is new work.p.pick generic map (t => real);
  function sm is new work.p.sum generic map (t => boolean);
  function hd generic (type t) generic map (t => natural) parameter (x : type is (<>); y : t) return t is
  begin
    return y;
  end function;
begin
  process
    variable i : integer;
    variable r : real;
  begin
    i := pi(true, 5);
    i := pi(1.0, 6);
    r := pr(i, 1.5);
    i := sm(1, 2, true);
    i := hd(character'('a'), 3);
    wait;
  end process;
end architecture;
