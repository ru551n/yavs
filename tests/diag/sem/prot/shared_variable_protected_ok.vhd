-- yavs-test: std=2008
-- expect: none
-- LRM 6.4.2.4
package p is
  type pt is protected
    procedure set (x : integer);
    impure function get return integer;
  end protected;
end package;
package body p is
  type pt is protected body
    variable v : integer;
    procedure set (x : integer) is begin v := x; end procedure;
    impure function get return integer is begin return v; end function;
  end protected body;
end package body;
use work.p.all;
entity e is end entity;
architecture a of e is
  shared variable sv : pt;
begin
end architecture;
