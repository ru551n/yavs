-- yavs-test: std=2019
-- expect: none
-- LRM 2019 5.6.2 (an alias in a protected type declaration may make a method of a private
-- variable visible; it needs no body in the protected body, L15)
package p is
  type inner is protected
    procedure set (x : integer);
    impure function get return integer;
  end protected;
  type outer is protected
    private variable iv : inner;
    alias set is iv.set [integer];
    alias get is iv.get [return integer];
    procedure twice (x : integer);
  end protected;
end package;
package body p is
  type inner is protected body
    variable v : integer := 0;
    procedure set (x : integer) is begin v := x; end procedure;
    impure function get return integer is begin return v; end function;
  end protected body;
  type outer is protected body
    procedure twice (x : integer) is begin set(2 * x); end procedure;
  end protected body;
end package body;
use work.p.all;
entity e is end entity;
architecture a of e is
  shared variable sv : outer;
begin
  process begin
    sv.set(1);
    sv.twice(sv.get);
    wait;
  end process;
end architecture;
