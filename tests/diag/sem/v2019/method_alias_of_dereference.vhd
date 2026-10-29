-- yavs-test: std=2019
-- expect: E1999 22:16 aliases of methods of objects not denoted by a simple or expanded name
-- LRM 2019 6.6.3 (forwarding through a prefix that is not an object name is not implemented;
-- a call of the poisoned alias reports nothing more)
package p is
  type inner is protected
    procedure set (x : integer);
  end protected;
  type inner_ptr is access inner;
end package;
package body p is
  type inner is protected body
    procedure set (x : integer) is begin null; end procedure;
  end protected body;
end package body;
use work.p.all;
entity e is end entity;
architecture a of e is
begin
  process
    variable ptr : inner_ptr := new inner;
    alias s is ptr.all.set [integer];
  begin
    s(1);
    wait;
  end process;
end architecture;
