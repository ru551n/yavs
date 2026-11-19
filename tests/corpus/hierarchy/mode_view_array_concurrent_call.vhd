-- difftest: top=mode_view_array_concurrent_call std=2019
-- A concurrent procedure call whose signal parameter has an array mode view: each element's
-- `ack` is read, its `req` driven; the call runs again when an `ack` changes.
package mode_view_array_concurrent_call_pkg is
  type sub_t is record
    req : natural;
    ack : natural;
  end record;

  view sub_m of sub_t is
    req : out;
    ack : in;
  end view;

  type arr_t is array (0 to 1) of sub_t;
end package;

use work.mode_view_array_concurrent_call_pkg.all;

entity mode_view_array_concurrent_call is end entity;

architecture a of mode_view_array_concurrent_call is
  procedure cross (signal m : view (sub_m) of arr_t) is
  begin
    m(0).req <= m(1).ack + 1;
    m(1).req <= m(0).ack * 2;
  end procedure;

  signal ar : arr_t;
begin
  u1 : cross(ar);

  check : process is
  begin
    assert ar(0).req = 0 and ar(1).req = 0;
    wait for 0 ns;
    assert ar(0).req = 1 and ar(1).req = 0 report "after the first delta" severity failure;
    ar(0).ack <= 5;
    wait for 1 ns;
    assert ar(0).req = 1 and ar(1).req = 10 report "after ack 0 changed" severity failure;
    ar(1).ack <= 3;
    wait for 1 ns;
    assert ar(0).req = 4 and ar(1).req = 10 report "after ack 1 changed" severity failure;
    report "ok";
    wait;
  end process;
end architecture;
