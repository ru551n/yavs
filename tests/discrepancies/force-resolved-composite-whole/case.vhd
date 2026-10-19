-- LRM 14.7.3.2, 14.7.3.3: `force in` of a record signal resolved as a whole sets its effective
-- value (the resolution function sees only the drivers); `release out` recomputes the driving
-- value from the drivers at once.
package pkg is
  type rec_u is record a : integer; n : integer; end record;
  type rec_drv is array (natural range <>) of rec_u;
  function sum(v : rec_drv) return rec_u;
  subtype rec_t is sum rec_u;
end package;
package body pkg is
  function sum(v : rec_drv) return rec_u is
    variable r : rec_u := (0, 0);
  begin
    for k in v'range loop
      r := (r.a + v(k).a, r.n + v(k).n);
    end loop;
    return r;
  end function;
end package body;
use work.pkg.all;
entity top is end entity;
architecture a of top is
  signal s : rec_t := (0, 0);
begin
  s <= (1, 1);
  process begin
    s <= (0, 2);
    wait for 1 ns;
    s <= force (5, 40);
    wait for 1 ns;
    report "RES force in " & integer'image(s.a) & " " & integer'image(s.n);
    s <= release;
    wait for 1 ns;
    s <= force out (7, 9);
    wait for 1 ns;
    report "RES force out " & integer'image(s.a) & " " & integer'image(s.n);
    s <= release out;
    wait for 1 ns;
    report "RES release out " & integer'image(s.a) & " " & integer'image(s.n);
    wait;
  end process;
end architecture;
