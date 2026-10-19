-- yavs-test: std=2019
-- expect: none
-- LRM 2019 5.6.2, 6.6.3 (an alias of a method of a shared variable or a variable; calls
-- through it run on that object)
package p is
  type inner is protected
    procedure set (x : integer);
    impure function get return integer;
  end protected;
end package;
package body p is
  type inner is protected body
    variable v : integer := 0;
    procedure set (x : integer) is begin v := x; end procedure;
    impure function get return integer is begin return v; end function;
  end protected body;
end package body;
use work.p.all;
package q is
  shared variable psv : inner;
  alias pset is psv.set [integer];
end package;
use work.p.all, work.q.all;
entity e is end entity;
architecture a of e is
  shared variable sv : inner;
  alias s is sv.set [integer];
  alias g is sv.get [return integer];
  alias pg is work.q.psv.get [return integer];
begin
  process
    variable lv : inner;
    alias ls is lv.set [integer];
  begin
    s(1);
    pset(g);
    ls(pg + 1);
    wait;
  end process;
end architecture;
