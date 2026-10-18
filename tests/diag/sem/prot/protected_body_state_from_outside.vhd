-- yavs-test: std=2008
-- expect: E1105 23:13 `v` is not in the visible part of protected type `pt`
-- LRM 5.6.3
package p is
  type pt is protected
    procedure set (x : integer);
  end protected;
end package;
package body p is
  type pt is protected body
    variable v : integer;
    procedure set (x : integer) is begin v := x; end procedure;
  end protected body;
end package body;
use work.p.all;
entity e is end entity;
architecture a of e is
  shared variable sv : pt;
begin
  process
    variable i : integer;
  begin
    i := sv.v;
    wait;
  end process;
end architecture;
