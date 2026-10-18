-- yavs-test: std=2019
-- expect: E1105 27:8 `iv` is a private variable of `outer`
-- LRM 2019 5.6.2
package p is
  type inner is protected
    procedure set (x : integer);
  end protected;
  type outer is protected
    private variable iv : inner;
    procedure go;
  end protected;
end package;
package body p is
  type inner is protected body
    procedure set (x : integer) is begin null; end procedure;
  end protected body;
  type outer is protected body
    procedure go is begin iv.set(1); end procedure;
  end protected body;
end package body;
use work.p.all;
entity e is end entity;
architecture a of e is
  shared variable sv : outer;
begin
  process begin
    sv.iv.set(1);
    wait;
  end process;
end architecture;
