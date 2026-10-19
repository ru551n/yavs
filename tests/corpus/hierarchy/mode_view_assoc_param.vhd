-- difftest: top=mode_view_assoc_param std=2019
-- 2019 individual association of a signal parameter with a mode view (6.5.7.1, 6.5.2): each
-- element of the formal is associated on its own, at any level of a nested view, in any order.
-- The `in` elements' actuals have no other driver, so only the driven elements matter.
package mode_view_assoc_param_pkg is
  type inner_t is record a : integer; b : bit; end record;
  view inner_m of inner_t is a : out; b : in; end view;
  type outer_t is record i : inner_t; v : bit; k : integer; end record;
  view outer_m of outer_t is i : view inner_m; v : in; k : out; end view;
  procedure put (signal o : view outer_m; n : integer);
end package;
package body mode_view_assoc_param_pkg is
  procedure put (signal o : view outer_m; n : integer) is
  begin
    report "put " & integer'image(n) & " v=" & bit'image(o.v) & " b=" & bit'image(o.i.b);
    o.i.a <= n;
    o.k <= n + 1;
  end procedure;
end package body;

use work.mode_view_assoc_param_pkg.all;
entity mode_view_assoc_param is end;
architecture t of mode_view_assoc_param is
  signal s : outer_t := ((0, '1'), '0', 0);
  signal sv : bit := '1';
  signal i2 : inner_t := (0, '1');
  signal k : integer := 0;
begin
  process begin
    put(o.i => s.i, o.v => s.v, o.k => s.k, n => 1);
    wait for 1 ns;
    report "s " & integer'image(s.i.a) & " " & integer'image(s.k);
    put(o.k => k, o.i => i2, o.v => sv, n => 5);
    wait for 1 ns;
    report "free " & integer'image(i2.a) & " " & integer'image(k);
    put(n => 9, o.v => s.v, o.i => s.i, o.k => s.k);
    wait for 1 ns;
    report "s " & integer'image(s.i.a) & " " & integer'image(s.k);
    wait;
  end process;
end;
