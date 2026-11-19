-- Out ports of a subtype resolved by a function of the design, on an unresolved actual, one
-- source per element: the function runs, also when it is named `resolved` (not the library's).
library ieee; use ieee.std_logic_1164.all;
package cycle_ownres_other_pkg is
  -- One driver: the inverse of its value, so a collapsed port would show the driver's own value.
  function inv_res(s : std_ulogic_vector) return std_ulogic;
  subtype inv_vec is (inv_res) std_ulogic_vector;
end package;
package body cycle_ownres_other_pkg is
  function inv_res(s : std_ulogic_vector) return std_ulogic is
    variable r : std_ulogic := '0';
  begin
    for i in s'range loop
      r := r xor s(i);
    end loop;
    return not r;
  end function;
end package body;

library ieee; use ieee.std_logic_1164.all; use work.cycle_ownres_other_pkg.all;
entity cycle_ownres_other_dut is
  port (clk : in std_ulogic; q : out inv_vec(3 downto 0));
end;
architecture rtl of cycle_ownres_other_dut is
  signal c : std_ulogic_vector(3 downto 0) := "0000";
begin
  process (clk) begin
    if rising_edge(clk) then
      c <= c(2 downto 0) & not c(3);
    end if;
  end process;
  g : for i in 0 to 3 generate
    q(i) <= c(i);
  end generate;
end;

library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all; use work.cycle_ownres_other_pkg.all;
entity cycle_ownres_other is end;
architecture tb of cycle_ownres_other is
  signal clk : std_ulogic := '0';
  signal q : std_ulogic_vector(3 downto 0);
begin
  u : entity work.cycle_ownres_other_dut port map (clk => clk, q => q);
  clk <= not clk after 5 ns when now < 60 ns else clk;
  process (clk) begin
    if falling_edge(clk) then
      report "q=" & to_string(q);
    end if;
  end process;
end;

library ieee; use ieee.std_logic_1164.std_ulogic; use ieee.std_logic_1164.std_ulogic_vector; use ieee.std_logic_1164."xor"; use ieee.std_logic_1164."not";
package cycle_ownres_named_pkg is
  -- One driver: the inverse of its value, so a collapsed port would show the driver's own value.
  function resolved(s : std_ulogic_vector) return std_ulogic;
  subtype inv_vec is (resolved) std_ulogic_vector;
end package;
package body cycle_ownres_named_pkg is
  function resolved(s : std_ulogic_vector) return std_ulogic is
    variable r : std_ulogic := '0';
  begin
    for i in s'range loop
      r := r xor s(i);
    end loop;
    return not r;
  end function;
end package body;

library ieee; use ieee.std_logic_1164.std_ulogic; use ieee.std_logic_1164.std_ulogic_vector; use ieee.std_logic_1164.rising_edge; use ieee.std_logic_1164."not"; use work.cycle_ownres_named_pkg.all;
entity cycle_ownres_named_dut is
  port (clk : in std_ulogic; q : out inv_vec(3 downto 0));
end;
architecture rtl of cycle_ownres_named_dut is
  signal c : std_ulogic_vector(3 downto 0) := "0000";
begin
  process (clk) begin
    if rising_edge(clk) then
      c <= c(2 downto 0) & not c(3);
    end if;
  end process;
  g : for i in 0 to 3 generate
    q(i) <= c(i);
  end generate;
end;

library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all; use work.cycle_ownres_named_pkg.all;
entity cycle_ownres_named is end;
architecture tb of cycle_ownres_named is
  signal clk : std_ulogic := '0';
  signal q : std_ulogic_vector(3 downto 0);
begin
  u : entity work.cycle_ownres_named_dut port map (clk => clk, q => q);
  clk <= not clk after 5 ns when now < 60 ns else clk;
  process (clk) begin
    if falling_edge(clk) then
      report "q=" & to_string(q);
    end if;
  end process;
end;
