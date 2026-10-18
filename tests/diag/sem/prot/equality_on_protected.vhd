-- yavs-test: std=2008
-- expect: E1905 22:10 operator `=` is not defined for protected type `pt`
-- LRM 5.6.2
package p is
  type pt is protected
    procedure set (x : integer);
  end protected;
end package;
package body p is
  type pt is protected body
    procedure set (x : integer) is begin null; end procedure;
  end protected body;
end package body;
use work.p.all;
entity e is end entity;
architecture a of e is
  shared variable a1, a2 : pt;
begin
  process
    variable b : boolean;
  begin
    b := a1 = a2;
    wait;
  end process;
end architecture;
