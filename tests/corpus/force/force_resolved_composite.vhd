-- difftest: top=force_resolved_composite std=2008
-- Force of a record signal resolved as a whole (#68): the resolution function still sees the
-- drivers' values (LRM 14.7.3.2), `force in` overrides the effective value and `force out` the
-- driving value of every element; release restores the resolved value. A member of such a
-- signal cannot be forced (LRM 10.5.2.1, E1362).
library ieee; use ieee.std_logic_1164.all;
package frc_pkg is
  type rec_u is record trigger : std_logic; n : integer; end record;
  type rec_drv is array (natural range <>) of rec_u;
  function sum(v : rec_drv) return rec_u;
  subtype rec_t is sum rec_u;
end package;
package body frc_pkg is
  function sum(v : rec_drv) return rec_u is
    variable r : rec_u := ('0', 0);
  begin
    for k in v'range loop
      r.n := r.n + v(k).n;
      if v(k).trigger = '1' then r.trigger := '1'; end if;
    end loop;
    return r;
  end function;
end package body;
library ieee; use ieee.std_logic_1164.all; use work.frc_pkg.all;
entity force_resolved_composite is end entity;
architecture a of force_resolved_composite is
  signal s : rec_t := ('0', 0);
begin
  p1 : process begin
    s <= ('0', 1);
    wait for 2 ns;
    s <= ('1', 1);
    wait;
  end process;
  p2 : process begin
    s <= ('0', 2);
    wait for 1 ns;
    report "resolved " & std_logic'image(s.trigger) & " " & integer'image(s.n);
    assert s = ('0', 3) severity failure;
    s <= force ('0', 40);
    wait for 1.5 ns;
    report "forced " & std_logic'image(s.trigger) & " " & integer'image(s.n);
    assert s = ('0', 40) severity failure;
    s <= release;
    wait for 1 ns;
    report "released " & std_logic'image(s.trigger) & " " & integer'image(s.n);
    assert s = ('1', 3) severity failure;
    s <= force out ('0', 9);
    wait for 1 ns;
    report "forced out " & std_logic'image(s.trigger) & " " & integer'image(s.n);
    assert s = ('0', 9) severity failure;
    s <= release out;
    wait for 1 ns;
    report "released out " & std_logic'image(s.trigger) & " " & integer'image(s.n);
    assert s = ('1', 3) severity failure;
    wait;
  end process;
end architecture;
