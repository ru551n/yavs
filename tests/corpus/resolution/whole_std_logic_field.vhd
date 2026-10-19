-- difftest: top=whole_std_logic_field std=2008
-- A record resolved as a whole with a std_logic field, the resolution function in a package
-- (#328): the field's own resolution is not used (LRM 14.7.3.1).
library ieee; use ieee.std_logic_1164.all;
package whole_sl_pkg is
  type rec_u is record trigger : std_logic; idx : integer; end record;
  type rec_drv is array (natural range <>) of rec_u;
  function resolved(v : rec_drv) return rec_u;
  subtype rec_t is resolved rec_u;
end package;
package body whole_sl_pkg is
  function resolved(v : rec_drv) return rec_u is
  begin
    return v(v'low);
  end function;
end package body;
library ieee; use ieee.std_logic_1164.all; use work.whole_sl_pkg.all;
entity whole_std_logic_field is end entity;
architecture a of whole_std_logic_field is
  signal s : rec_t := (trigger => 'L', idx => -1);
begin
  process begin
    s.idx <= 3;
    wait for 1 ns;
    report std_logic'image(s.trigger) & " " & integer'image(s.idx);
    assert s = ('L', 3) severity failure;
    wait;
  end process;
end architecture;
