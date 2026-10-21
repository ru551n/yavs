-- yavs-test: std=2008
-- expect: none
-- LRM 10.6.2.1, 10.5.2.1 (OSVVM ClockResetPkg: an aggregate target selects the composite overload; sim1 and sim2 accept)
package p is
  impure function pick (n : integer) return integer;
  impure function pick (n : integer) return integer_vector;
end package;
package body p is
  impure function pick (n : integer) return integer is
  begin
    return n;
  end function;
  impure function pick (n : integer) return integer_vector is
  begin
    return (n, n + 1);
  end function;
end package body;
use work.p.all;
entity e is end entity;
architecture a of e is
  signal s1, s2 : integer;
begin
  process
    variable a, b : integer;
  begin
    (a, b) := pick(1);
    a := pick(2);
    (s1, s2) <= pick(3);
    wait;
  end process;
end architecture;
