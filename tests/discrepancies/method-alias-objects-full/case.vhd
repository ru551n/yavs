-- (from the differential corpus)
-- 2019 aliases of methods of a shared variable in a package and in an architecture (also
-- through an object alias) and of a process variable (5.6.2, 6.6.3): calls through the
-- alias run on the aliased object. The reference simulators crash on or reject such calls
-- (lrm-checks L15); the .expected is by hand, the result of the same calls written as
-- selected names (`sv.add(3)`).
package method_alias_objects_pkg is
  type counter_t is protected
    procedure add (n : integer);
    impure function get return integer;
  end protected;
end package;
package body method_alias_objects_pkg is
  type counter_t is protected body
    variable c : integer := 0;
    procedure add (n : integer) is begin c := c + n; end procedure;
    impure function get return integer is begin return c; end function;
  end protected body;
end package body;
use work.method_alias_objects_pkg.all;
package method_alias_objects_sv is
  shared variable total : counter_t;
  alias total_add is total.add [integer];
end package;
use work.method_alias_objects_pkg.all;
use work.method_alias_objects_sv.all;
entity method_alias_objects is end entity;
architecture a of method_alias_objects is
  shared variable sv : counter_t;
  alias sv_add is sv.add [integer];
  alias sv_get is sv.get [return integer];
  alias total_get is work.method_alias_objects_sv.total.get [return integer];
  alias sv2 is sv;
  alias sv2_add is sv2.add [integer];
begin
  p1 : process
    variable lv : counter_t;
    alias lv_add is lv.add [integer];
    alias lv_get is lv.get [return integer];
  begin
    sv_add(3);
    total_add(10);
    lv_add(sv_get + 1);
    lv_add(2);
    report "sv=" & integer'image(sv.get) & " lv=" & integer'image(lv_get);
    wait for 1 ns;
    report "total=" & integer'image(total_get);
    wait;
  end process;
  p2 : process
  begin
    wait for 2 ns;
    total_add(5);
    sv2_add(4);
    wait for 1 ns;
    report "sv=" & integer'image(sv_get) & " total=" & integer'image(total.get);
    wait;
  end process;
end architecture;
