-- yavs-test: std=2008
-- expect: E1905 20:5 protected type `pt` cannot be assigned
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
  process begin
    a1 := a2;
    wait;
  end process;
end architecture;
