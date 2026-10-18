-- yavs-test: std=2008
-- expect: E1301 20:7 no method `set` matches the call
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
  shared variable v : pt;
begin
  process begin
    v.set(true);
    wait;
  end process;
end architecture;
