-- difftest: top=package_alloc std=2008
-- Access values made by package subprograms and dereferenced by several processes. A package
-- constant cannot hold one (LRM 6.4.2.2, E1223, #198), so each process owns its objects.
package alloc_pkg is
  type int_ptr is access integer;
  type pair is record a, b : integer; end record;
  type pair_ptr is access pair;
  constant scale : integer := 10;
  impure function make (v : integer) return int_ptr;
  impure function make_pair (a, b : integer) return pair_ptr;
  procedure bump (variable p : inout int_ptr; by : integer);
end package;

package body alloc_pkg is
  impure function make (v : integer) return int_ptr is
  begin
    return new integer'(v * scale);
  end function;

  impure function make_pair (a, b : integer) return pair_ptr is
  begin
    return new pair'(a, b);
  end function;

  procedure bump (variable p : inout int_ptr; by : integer) is
  begin
    p.all := p.all + by;
  end procedure;
end package body;

use work.alloc_pkg.all;
entity package_alloc is end;
architecture t of package_alloc is
begin
  p1: process
    variable x : int_ptr := make(4);
    variable q : pair_ptr;
  begin
    bump(x, 2);
    q := make_pair(x.all, 7);
    assert q.a = 42 and q.b = 7 severity failure;
    report "p1 " & integer'image(x.all) & " " & integer'image(q.a + q.b);
    deallocate(x);
    deallocate(q);
    assert x = null and q = null severity failure;
    wait;
  end process;

  p2: process
    variable y : int_ptr;
  begin
    wait for 1 ns;
    y := make(-3);
    for i in 1 to 5 loop
      bump(y, i);
    end loop;
    assert y.all = -15 severity failure;
    report "p2 " & integer'image(y.all);
    wait;
  end process;
end;
