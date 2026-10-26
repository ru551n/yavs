-- (found by tools/waveq on the UVVM AXI-Stream testbench, #781) The signature of a subprogram in
-- 'instance_name and 'path_name of an object declared in it (16.2.5): parameters of a subtype
-- (std_logic, natural). The UVVM BFMs seed their random streams from this string.
library ieee; use ieee.std_logic_1164.all;
package signame_pkg is
  type pv is array (natural range <>) of positive;
  procedure show(constant scope : in string; signal s : in std_logic; n : natural);
end package;
package body signame_pkg is
  procedure show(constant scope : in string; signal s : in std_logic; n : natural) is
    variable v_seeds : pv(0 to 1);
  begin
    report "RES instance_name " & v_seeds'instance_name;
    report "RES path_name " & v_seeds'path_name;
  end procedure;
end package body;

library ieee; use ieee.std_logic_1164.all; use work.signame_pkg.all;
entity signame is end entity;
architecture a of signame is
  signal clk : std_logic := '0';
begin
  process begin show("x", clk, 1); wait; end process;
end architecture;
