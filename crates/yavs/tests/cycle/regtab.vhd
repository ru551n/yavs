-- A register table given as a generic array of records (a register file's description), against
-- the normal mode (`W` replaced by a width): a loop over the table's range assigns, per register
-- of a mode, a slice whose length is a field of its element.
library ieee;
use ieee.std_logic_1164.all;
package cycle_regtab_pkg is
  type reg_t is record
    mode : natural;
    width : positive;
  end record;
  type regs_t is array (natural range <>) of reg_t;
  type vec_t is array (natural range <>) of std_ulogic_vector(W + 7 downto 0);
end package;

library ieee;
use ieee.std_logic_1164.all;
use work.cycle_regtab_pkg.all;
entity cycle_regtab_u is
  generic (registers : regs_t);
  port (vals : in vec_t(registers'range);
        down : out vec_t(registers'range) := (others => (others => '0')));
end entity;
architecture a of cycle_regtab_u is
begin
  assign_down : process (all)
  begin
    for i in registers'range loop
      if registers(i).mode = 1 then
        down(i)(registers(i).width - 1 downto 0) <= vals(i)(registers(i).width - 1 downto 0);
      end if;
    end loop;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use work.cycle_regtab_pkg.all;
entity cycle_regtab is
end entity;
architecture t of cycle_regtab is
  constant REGS : regs_t(0 to 3) := ((1, 1), (0, W), (1, W + 8), (1, W));
  signal vals, down : vec_t(0 to 3) := (others => (others => '0'));
begin
  u : entity work.cycle_regtab_u generic map (registers => REGS) port map (vals => vals, down => down);
  main : process
    variable s : std_ulogic_vector(15 downto 0) := x"ACE1";
  begin
    for k in 1 to 10 loop
      for r in 0 to 3 loop
        s := s(14 downto 0) & (s(15) xor s(13) xor s(12) xor s(10));
        vals(r) <= vals(r)(W + 6 downto 0) & s(0);
      end loop;
      wait for 10 ns;
      report "k=" & integer'image(k) & " " & to_hstring(down(0)) & " " & to_hstring(down(1)) & " "
        & to_hstring(down(2)) & " " & to_hstring(down(3));
    end loop;
    wait;
  end process;
end architecture;
