-- difftest: top=mode_view_concurrent_call std=2019
-- A concurrent procedure call with mode view signal parameters is sensitive to the elements
-- the view reads (in and inout), not to the ones it only drives: two calls that feed each
-- other through the elements of a record.
package mode_view_concurrent_call_pkg is
  type rec_t is record
    x, y : natural;
  end record;

  view in_view of rec_t is
    x : in;
    y : out;
  end view;

  view out_view of rec_t is
    x : out;
    y : in;
  end view;
end package;

use work.mode_view_concurrent_call_pkg.all;

entity mode_view_concurrent_call is end entity;

architecture a of mode_view_concurrent_call is
  procedure update1 (signal i : view in_view; signal o : view out_view) is
  begin
    o.x <= i.x + 1;
    i.y <= o.y + 5;
  end procedure;

  procedure update2 (signal i : view in_view'converse; signal o : view out_view'converse) is
  begin
    i.x <= 7;
    o.y <= 12;
  end procedure;

  signal i, o : rec_t;
begin
  u1 : update1(i, o);

  u2 : process is
  begin
    wait for 0 ns;
    update2(i, o);
    wait;
  end process;

  check : process is
  begin
    assert i = (0, 0);
    assert o = (0, 0);
    wait for 0 ns;
    assert i = (0, 5) report "after the first delta" severity failure;
    assert o = (1, 0) report "after the first delta" severity failure;
    wait for 1 ns;
    assert i = (7, 17) report "after the reads changed" severity failure;
    assert o = (8, 12) report "after the reads changed" severity failure;
    report "ok";
    wait;
  end process;
end architecture;
