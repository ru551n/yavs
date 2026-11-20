-- difftest: top=object_range_once_2019 std=2019 ref=hand
-- ref=hand: sim1 evaluates the range of an object's anonymous subtype again at every check and
-- attribute (discrepancy `object-range-bound-variable`); sim2 agrees with this output on the
-- lines it runs (it has no `'subtype` of an object). The range of an object's anonymous subtype
-- is evaluated once, when its declaration is elaborated (LRM 14.4.2.1), in a process, an
-- architecture and a package, and a loop parameter's when the loop starts (LRM 10.10, #1174).
package object_range_once_2019_cnt is
  type counter is protected
    procedure inc;
    impure function get return natural;
  end protected;
end package;
package body object_range_once_2019_cnt is
  type counter is protected body
    variable v : natural := 0;
    procedure inc is begin v := v + 1; end procedure;
    impure function get return natural is begin return v; end function;
  end protected body;
end package body;
use work.object_range_once_2019_cnt.all;
package object_range_once_2019_f is
  shared variable c : counter;
  impure function f (x : integer) return integer;
end package;
package body object_range_once_2019_f is
  impure function f (x : integer) return integer is begin c.inc; return x; end function;
end package body;
use work.object_range_once_2019_f.all;
package object_range_once_2019_p is
  signal psig : integer range 0 to f(4) := 0;
  constant pc : integer range 0 to f(8) := 1;
end package;
use work.object_range_once_2019_f.all;
use work.object_range_once_2019_p.all;
entity object_range_once_2019 is end entity;
architecture a of object_range_once_2019 is
  signal s : integer range 0 to f(7) := 0;
  constant ac : integer range 0 to f(9) := 2;
begin
  process
    variable k : integer := 9;
    variable q : integer range 0 to k := 0;
    variable q2 : q'subtype := 0;
    constant cq : integer range 0 to k := 3;
    variable r : integer range f(5) to 10 := 5;
    variable m : integer := 0;
    -- A formal's range that reads an earlier formal (2019) is the call's.
    procedure lim (n : integer; x : inout integer range 0 to n) is begin x := n; end procedure;
  begin
    report "decl calls=" & integer'image(c.get);
    assert c.get = 5 severity failure;
    k := 1;
    q := 5;
    q2 := 6;
    r := 6;
    report "q'subtype'high=" & integer'image(q'subtype'high) & " q2'subtype'high="
      & integer'image(q2'subtype'high) & " cq'subtype'high=" & integer'image(cq'subtype'high)
      & " r'subtype'low=" & integer'image(r'subtype'low);
    for i in 0 to k loop
      k := 5;
      m := m * 10 + i'subtype'high;
    end loop;
    report "loop m=" & integer'image(m);
    assert m = 11 severity failure;
    lim(4, m);
    lim(12, m);
    report "lim m=" & integer'image(m);
    s <= 6;
    psig <= 3;
    report "s'subtype'high=" & integer'image(s'subtype'high) & " psig'subtype'high="
      & integer'image(psig'subtype'high) & " ac'subtype'high=" & integer'image(ac'subtype'high)
      & " pc'subtype'high=" & integer'image(pc'subtype'high);
    wait for 1 ns;
    report "s=" & integer'image(s) & " psig=" & integer'image(psig) & " calls=" & integer'image(c.get);
    assert c.get = 5 severity failure;
    wait;
  end process;
end architecture;
