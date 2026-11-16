-- difftest: top=user_resolved_named std=2008
-- A resolution function of the design named `resolved` on std_ulogic is called, not the
-- library's (#1162): on a signal and on an out port, scalar and vector, with one source and with
-- two. The function inverts the xor of its sources, so a library resolution gives other values.
-- The library's own `resolved` (std_logic) still resolves a bus at the end. The second reference
-- simulator does not call the function on the out port (tests/discrepancies/out-port-resolution-unresolved-actual).
library ieee;
use ieee.std_logic_1164.std_ulogic;
use ieee.std_logic_1164.std_ulogic_vector;
use ieee.std_logic_1164."xor";
use ieee.std_logic_1164."not";

package user_resolved_pkg is
  function resolved(s : std_ulogic_vector) return std_ulogic;
  subtype inv is resolved std_ulogic;
  subtype inv_vec is (resolved) std_ulogic_vector;
end package;

package body user_resolved_pkg is
  function resolved(s : std_ulogic_vector) return std_ulogic is
    variable r : std_ulogic := '0';
  begin
    for i in s'range loop
      r := r xor s(i);
    end loop;
    return not r;
  end function;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use work.user_resolved_pkg.all;

-- One source per element, on an out port of the resolved vector subtype.
entity user_resolved_dut is
  port (d : in std_ulogic_vector(3 downto 0); q : out inv_vec(3 downto 0); o : out inv);
end entity;

architecture rtl of user_resolved_dut is
begin
  g : for i in 0 to 3 generate
    q(i) <= d(i);
  end generate;
  o <= d(0);
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use work.user_resolved_pkg.all;

entity user_resolved_named is
end entity;

architecture tb of user_resolved_named is
  signal one, two : inv;
  signal vec      : inv_vec(3 downto 0);
  signal d        : std_ulogic_vector(3 downto 0) := "0000";
  signal q        : std_ulogic_vector(3 downto 0);
  signal o        : std_ulogic;
  signal bus_std  : std_logic;
  signal bus_vec  : std_logic_vector(1 downto 0);
begin
  one <= '1';
  two <= '1';
  two <= '0';
  vec <= "1010";

  u : entity work.user_resolved_dut port map (d => d, q => q, o => o);

  bus_std <= 'Z';
  bus_std <= '1';
  bus_vec <= "1Z";
  bus_vec <= "Z0";

  process begin
    d <= "1100";
    wait for 1 ns;
    report "one=" & to_string(one) & " two=" & to_string(two) & " vec=" & to_string(vec);
    report "q=" & to_string(q) & " o=" & to_string(o);
    report "bus_std=" & to_string(bus_std) & " bus_vec=" & to_string(bus_vec);
    assert one = '0' report "one" severity failure;
    assert two = '0' report "two" severity failure;
    assert vec = "0101" report "vec" severity failure;
    assert q = "0011" report "q" severity failure;
    assert o = '1' report "o" severity failure;
    assert bus_std = '1' report "bus_std" severity failure;
    assert bus_vec = "10" report "bus_vec" severity failure;
    d <= "0110";
    wait for 1 ns;
    report "q=" & to_string(q) & " o=" & to_string(o);
    assert q = "1001" report "q2" severity failure;
    assert o = '1' report "o2" severity failure;
    wait;
  end process;
end architecture;
