-- difftest: top=package_record_signals std=2008
-- Package signals whose type has records in it: read, driven whole, by field and by element
-- from processes of several instances, waited on, passed to signal parameters and named in
-- attributes (#215).
package prs_pkg is
  type cell is record
    ok : boolean;
    n  : natural;
  end record;
  type cell_vec is array (natural range <>) of cell;
  signal g  : cell := (false, 3);
  signal gv : cell_vec(0 to 1) := ((true, 1), (false, 2));
  function image(c : cell) return string;
  procedure put(signal x : out cell; v : natural);
end package;

package body prs_pkg is
  function image(c : cell) return string is
  begin
    return "(" & boolean'image(c.ok) & "," & integer'image(c.n) & ")";
  end function;

  procedure put(signal x : out cell; v : natural) is
  begin
    x <= (v mod 2 = 0, v);
  end procedure;
end package body;

use work.prs_pkg.all;

entity prs_writer is
  generic (k : natural);
end entity;

architecture rtl of prs_writer is
begin
  process
  begin
    wait for k * 1 ns;
    gv(k - 1).n <= 10 * k;
    wait;
  end process;
end architecture;

use work.prs_pkg.all;

entity package_record_signals is
end entity;

architecture test of package_record_signals is
begin
  w1 : entity work.prs_writer generic map (k => 1);
  w2 : entity work.prs_writer generic map (k => 2);

  main : process
  begin
    report "g " & image(g) & " gv " & image(gv(0)) & image(gv(1));
    g <= (true, 5);
    wait for 0 ns;
    report "g " & image(g) & " event " & boolean'image(g'event)
      & " last " & image(g'last_value);
    put(g, 8);
    wait for 0 ns;
    report "g " & image(g) & " g.n'event " & boolean'image(g.n'event)
      & " g.ok'event " & boolean'image(g.ok'event);
    assert g = (true, 8) severity failure;
    wait for 5 ns;
    report "gv " & image(gv(0)) & image(gv(1));
    assert gv = ((true, 10), (false, 20)) severity failure;
    wait;
  end process;

  watch : process (gv) is
  begin
    report "gv changed: " & image(gv(0)) & image(gv(1))
      & " gv(1)'event " & boolean'image(gv(1)'event);
  end process;
end architecture;
