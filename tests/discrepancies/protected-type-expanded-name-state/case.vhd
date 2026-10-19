-- LRM 8.3, 12.1: inside a protected body, `pt.n` with the protected type as prefix denotes the
-- body's state, also where a parameter `n` hides it (#250)
package p is
  type pt is protected
    procedure set (n : integer);
    impure function get return integer;
    impure function twice return integer;
  end protected;
end package;
package body p is
  type pt is protected body
    variable n : integer := 0;
    procedure set (n : integer) is begin pt.n := n; end procedure;
    impure function get return integer is begin return pt.n; end function;
    impure function twice return integer is begin return 2 * get; end function;
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
