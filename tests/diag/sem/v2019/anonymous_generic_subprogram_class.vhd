-- yavs-test: std=2019
-- expect: E1301 16:27 no function `sm` matches the call
-- LRM 2019 6.5.3: the actual of a parameter of anonymous integer type is of an integer type
package p is
  function sum generic (type t) parameter (a : type is range <>; b : a'subtype; c : t) return a'subtype;
end package;
package body p is
  function sum generic (type t) parameter (a : type is range <>; b : a'subtype; c : t) return a'subtype is
  begin
    return a + b;
  end function;
end package body;
entity top is end;
architecture a of top is
  function sm is new work.p.sum generic map (t => boolean);
  constant k : integer := sm(1.5, 2.5, true);
begin
end architecture;
