-- difftest: top=element_attr_open std=2008
-- 'element attributes of objects whose element bounds are open (#235, #243): the
-- bounds come from the object (an unconstrained port, a subprogram parameter, an alias,
-- a variable, a signal), for 'length, 'left, 'right, 'low, 'high and 'range.
library ieee;
use ieee.std_logic_1164.all;
package element_attr_open_pkg is
  type ram_t is array (natural range <>) of bit_vector;
  type sram_t is array (natural range <>) of std_logic_vector;
end;

library ieee;
use ieee.std_logic_1164.all;
use work.element_attr_open_pkg.all;

entity inner is
  port (p : in ram_t; q : in sram_t);
end;

architecture a of inner is
  constant w : natural := p'element'length;
  constant qw : natural := q'element'length;
  alias pa : ram_t is p;
  alias ea is <<signal .element_attr_open.sr : sram_t>>;
begin
  process
    variable s : string(1 to 3 * w);
    variable n : natural := 0;
  begin
    report "port " & integer'image(w) & " " & integer'image(p'element'left) & " "
      & integer'image(p'element'right) & " " & integer'image(p'element'low) & " "
      & integer'image(p'element'high) & " " & integer'image(pa'element'length);
    for i in p'element'range loop n := n + 1; end loop;
    report "range " & integer'image(n) & " " & integer'image(s'length);
    report "sport " & integer'image(qw) & " " & integer'image(q'element'left) & " "
      & integer'image(q'element'right);
    report "external " & integer'image(ea'element'length) & " " & integer'image(ea'element'left)
      & " " & integer'image(ea'element'right) & " " & integer'image(ea'length);
    wait;
  end process;
end;

library ieee;
use ieee.std_logic_1164.all;
use work.element_attr_open_pkg.all;

entity element_attr_open is end;

architecture t of element_attr_open is
  signal r : ram_t(0 to 3)(7 downto 0);
  signal sr : sram_t(1 to 2)(0 to 4);
  signal u : ram_t(0 to 1)(2 to 5);
  alias ua : ram_t is u;

  function show(m : ram_t) return string is
    variable s : string(1 to m'length * (m'element'length + 1));
  begin
    return integer'image(s'length) & " " & integer'image(m'element'left) & " "
      & integer'image(m'element'right) & " " & integer'image(m'element'high);
  end;
begin
  i0 : entity work.inner port map (p => r, q => sr);

  process
    variable v : ram_t(0 to 2)(15 downto 8);
    variable n : natural := 0;
  begin
    wait for 1 ns;
    report "variable " & integer'image(v'element'length) & " "
      & integer'image(v'element'left) & " " & integer'image(v'element'right) & " "
      & integer'image(v'element'low) & " " & integer'image(v'element'high);
    for i in v'element'range loop n := n + 1; end loop;
    report "vrange " & integer'image(n);
    report "signal " & integer'image(u'element'length) & " "
      & integer'image(u'element'left) & " " & integer'image(u'element'right) & " "
      & integer'image(ua'element'length) & " " & integer'image(ua'element'high);
    report "sig2 " & integer'image(sr'element'length) & " "
      & integer'image(sr'element'right);
    report "param " & show(v);
    report "param2 " & show(r);
    wait;
  end process;
end;
