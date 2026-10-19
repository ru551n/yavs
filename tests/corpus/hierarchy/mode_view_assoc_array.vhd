-- difftest: top=mode_view_assoc_array std=2019 ref=hand
-- 2019 individual association of nested and array mode views (6.5.7.1, 6.5.2), in port maps and
-- in subprogram calls, at every level of the view.
-- ref=hand: one reference simulator rejects formal parts below the first level in calls, the
-- other has no assignment to mode view elements in subprograms; the expected output is the
-- LRM result (the second one's, where it can run the case).
package mode_view_assoc_array_pkg is
  type inner_t is record a : integer; b : bit; end record;
  view inner_m of inner_t is a : out; b : in; end view;
  type outer_t is record i : inner_t; v : bit; k : integer; end record;
  view outer_m of outer_t is i : view inner_m; v : in; k : out; end view;
  type arr_t is array (0 to 1) of inner_t;
  procedure put (signal o : view outer_m; n : integer);
  procedure putarr (signal o : view (inner_m) of arr_t; n : integer);
end package;
package body mode_view_assoc_array_pkg is
  procedure put (signal o : view outer_m; n : integer) is
  begin
    report "put " & bit'image(o.v) & bit'image(o.i.b);
    o.i.a <= n; o.k <= n + 1;
  end procedure;
  procedure putarr (signal o : view (inner_m) of arr_t; n : integer) is
  begin
    report "arr " & bit'image(o(0).b) & bit'image(o(1).b);
    o(0).a <= n; o(1).a <= n + 1;
  end procedure;
end package body;
use work.mode_view_assoc_array_pkg.all;
entity mode_view_assoc_array_m is port (p : view outer_m; q : view (inner_m) of arr_t); end;
architecture a of mode_view_assoc_array_m is begin
  process begin
    put(p, 1);
    putarr(q, 4);
    report "m " & bit'image(p.v);
    wait;
  end process;
end;
use work.mode_view_assoc_array_pkg.all;
entity mode_view_assoc_array is end;
architecture t of mode_view_assoc_array is
  signal s : outer_t := ((0, '1'), '1', 0);
  signal ia : integer; signal ib : bit := '1'; signal sv : bit := '1'; signal k, k2 : integer;
  signal qa : arr_t := (others => (0, '1'));
  signal q0a, q1a, q0a2 : integer; signal qa2 : arr_t := (others => (0, '1')); signal q0b, q1b : bit := '1';
begin
  m1 : entity work.mode_view_assoc_array_m port map (p.i.a => ia, p.i.b => ib, p.v => sv, p.k => k, q(0).a => q0a, q(0).b => q0b, q(1) => qa(1));
  process begin
    wait for 1 ns;
    report integer'image(ia) & integer'image(k) & integer'image(q0a) & integer'image(qa(1).a);
    put(o.i => s.i, o.v => sv, o.k => k2, n => 3);
    putarr(o(0).a => q0a2, o(0).b => q0b, o(1) => qa2(1), n => 7);
    putarr(o(0) => qa2(0), o(1).a => q1a, o(1).b => q1b, n => 7);
    wait for 1 ns;
    report integer'image(ia) & integer'image(k) & integer'image(q0a) & integer'image(q1a);
    wait;
  end process;
end;
