-- LRM 8.3, 12.1: inside a protected body, `pt.get` with the protected type as prefix calls the
-- method on the current object (#250)
package p is
  type pt is protected
    procedure set (x : integer);
    impure function get return integer;
    impure function twice return integer;
  end protected;
end package;
package body p is
  type pt is protected body
    variable n : integer := 0;
    procedure set (x : integer) is begin n := x; end procedure;
    impure function get return integer is begin return n; end function;
    impure function twice return integer is begin return 2 * pt.get; end function;
  end protected body;
end package body;
use work.p.all;
entity top is end entity;
architecture a of top is
  shared variable sv : pt;
begin
  process
  begin
    sv.set(5);
    report "RES get=" & integer'image(sv.get) & " twice=" & integer'image(sv.twice);
    wait;
  end process;
end architecture;
