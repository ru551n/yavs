-- difftest: top=inout_port_user_resolution std=2008 ignore=order
-- An `inout` port of a subtype with a user resolution function that is not associative (the
-- mean, truncated): the port resolves its own two drivers first and is then one source of its
-- actual (LRM 14.7.3.2), so the actual is mean(mean(10, 20), 60) = 37, not mean(10, 20, 60) =
-- 30. Such a port keeps its edges; the values are the same with and without collapsing ports.
-- The second reference simulator resolves the three drivers together (30). ignore=order: the
-- two readers report in the same delta.
package iopu_pkg is
  type int_vec is array (natural range <>) of integer;
  function mean (v : int_vec) return integer;
  subtype mean_int is mean integer;
end package;

package body iopu_pkg is
  function mean (v : int_vec) return integer is
    variable s : integer := 0;
  begin
    if v'length = 0 then
      return 0;
    end if;
    for i in v'range loop
      s := s + v(i);
    end loop;
    return s / v'length;
  end function;
end package body;

use work.iopu_pkg.all;

entity iopu_leaf is
  port (p : inout mean_int := 0);
end;
architecture t of iopu_leaf is
begin
  d1 : p <= 10, 40 after 2 ns;
  d2 : p <= 20;
  rd : process (p)
  begin
    report "leaf p=" & integer'image(p);
  end process;
end;

use work.iopu_pkg.all;

entity inout_port_user_resolution is end;
architecture t of inout_port_user_resolution is
  signal s : mean_int := 0;
begin
  u : entity work.iopu_leaf port map (p => s);
  d : s <= 60;
  rd : process (s)
  begin
    report "top s=" & integer'image(s);
  end process;
  check : process
  begin
    wait for 1 ns;
    assert s = 37 report "s=" & integer'image(s) severity failure;
    wait for 2 ns;
    assert s = 45 report "s=" & integer'image(s) severity failure;
    report "done";
    wait;
  end process;
end;
