-- An assert that holds by what elaboration knows (a width check in a called function, as
-- hdl-modules' axi_pkg has) is no code in the region: the input of its combinational process
-- waits for the next edge instead of being settled at once.
library ieee;
use ieee.std_logic_1164.all;

entity cycle_foldassert_dut is
  port (clk : in std_ulogic; d : in std_ulogic_vector(7 downto 0); q : out std_ulogic_vector(7 downto 0));
end entity;

architecture rtl of cycle_foldassert_dut is
  function pack(x : std_ulogic_vector; w : natural) return std_ulogic_vector is
    variable r : std_ulogic_vector(w - 1 downto 0);
    variable hi : natural := 0;
  begin
    hi := w - 1;
    r(hi downto 0) := x(hi downto 0);
    assert hi = r'high report "pack: width" severity failure;
    return r;
  end function;
  signal p : std_ulogic_vector(7 downto 0);
begin
  comb : process (all)
  begin
    p <= pack(d, 8);
  end process;

  reg : process (clk)
  begin
    if rising_edge(clk) then
      q <= p;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity cycle_foldassert is
end entity;

architecture tb of cycle_foldassert is
  signal clk : std_ulogic := '0';
  signal d, q : std_ulogic_vector(7 downto 0) := x"00";
begin
  dut : entity work.cycle_foldassert_dut port map (clk => clk, d => d, q => q);
  main : process
  begin
    for k in 1 to 4 loop
      d <= std_ulogic_vector(to_unsigned(k, 8));
      wait for 2 ns;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 3 ns;
    end loop;
    report "q=" & to_hstring(q);
    wait;
  end process;
end architecture;
