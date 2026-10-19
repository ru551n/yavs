-- yavs-test: std=2019
-- expect: none
-- LCS-2016-049: a generic map in a call of an uninstantiated subprogram (functions,
-- procedures, concurrent procedure calls, expanded names, calls inside generic bodies)
package p is
  function ident generic (type t) parameter (x : t) return t;
end package;
package body p is
  function ident generic (type t) parameter (x : t) return t is begin return x; end function;
end package body;
use work.p.all;
entity e is end entity;
architecture a of e is
  procedure show generic (type t; function im (x : t) return string)
    parameter (x : t) is
  begin
    report im(x);
  end procedure;
  procedure hello generic (n : integer := 3) is begin report integer'image(n); end procedure;
  function img (x : integer) return string is begin return integer'image(x); end function;
  function outer generic (type u) parameter (x : u) return u is
  begin
    return ident generic map (t => u) (x);
  end function;
  function oi is new outer generic map (u => integer);
begin
  show generic map (t => integer, im => img) (1);
  process
  begin
    report img(ident generic map (t => integer) (4) + work.p.ident generic map (integer) (x => 5));
    show generic map (integer, img) (x => 6);
    hello generic map (n => 7);
    hello generic map (open);
    report img(oi(8));
    wait;
  end process;
end architecture;
