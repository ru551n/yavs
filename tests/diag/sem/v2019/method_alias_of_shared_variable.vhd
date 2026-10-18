-- yavs-test: std=2019
-- expect: E9999 18:14 aliases of methods of objects other than a private variable
-- LRM 2019 5.6.2, 6.6.3 (forwarding through any other object is not implemented)
package p is
  type inner is protected
    procedure set (x : integer);
  end protected;
end package;
package body p is
  type inner is protected body
    procedure set (x : integer) is begin null; end procedure;
  end protected body;
end package body;
use work.p.all;
entity e is end entity;
architecture a of e is
  shared variable sv : inner;
  alias s is sv.set [integer];
begin
end architecture;
