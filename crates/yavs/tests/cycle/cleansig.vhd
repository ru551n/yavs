-- --two-state: signals only two-state processes drive are read without a map; one forced or
-- driven from the testbench, with a metavalue start, resolved and driven 'Z', behind an inout
-- port, or a port the testbench feeds is still mapped ('1' and 'H' are 1, anything else 0).
-- `ones` counts the elements read as 1, and those read as 0 in hundreds.
library ieee;
use ieee.std_logic_1164.all;

entity cleansig_sub is
  port (b : inout std_logic_vector(3 downto 0));
end entity;

architecture rtl of cleansig_sub is
begin
  b <= "ZZ1Z";
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity cleansig_dut is
  port (
    clk : in std_ulogic;
    pin : in std_ulogic_vector(3 downto 0);
    n_ok, n_forced, n_tb, n_init, n_res, n_port, n_pin : out natural);
end entity;

architecture rtl of cleansig_dut is
  signal s_ok : std_ulogic_vector(3 downto 0);
  signal s_forced : std_ulogic_vector(3 downto 0) := "0000";
  signal s_tb : std_ulogic_vector(3 downto 0) := "0000";
  signal s_init : std_ulogic_vector(3 downto 0) := "U1X1";
  signal s_res : std_logic_vector(3 downto 0);
  signal s_port : std_logic_vector(3 downto 0);
  function ones(v : std_ulogic_vector) return natural is
    variable c : natural := 0;
  begin
    for i in v'range loop
      if v(i) = '1' then
        c := c + 1;
      elsif v(i) = '0' then
        c := c + 100;
      end if;
    end loop;
    return c;
  end function;
begin
  sub : entity work.cleansig_sub port map (s_port);
  s_res <= "Z1Z1";
  process (clk)
    variable c : integer_vector(0 to 6) := (others => 0);
  begin
    if rising_edge(clk) then
      s_ok <= not s_ok;
      c := (c(0) + ones(s_ok), c(1) + ones(s_forced), c(2) + ones(s_tb), c(3) + ones(s_init),
        c(4) + ones(std_ulogic_vector(s_res)), c(5) + ones(std_ulogic_vector(s_port)), c(6) + ones(pin));
      n_ok <= c(0);
      n_forced <= c(1);
      n_tb <= c(2);
      n_init <= c(3);
      n_res <= c(4);
      n_port <= c(5);
      n_pin <= c(6);
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity cleansig is
end entity;

architecture tb of cleansig is
  signal clk : std_ulogic := '0';
  signal n_ok, n_forced, n_tb, n_init, n_res, n_port, n_pin : natural;
  signal x : std_ulogic_vector(3 downto 0) := "X1U1";
  signal y : std_ulogic_vector(3 downto 0);
begin
  -- A port the testbench feeds through a concurrent copy (the kernel's own source).
  y <= x;
  dut : entity work.cleansig_dut port map (clk, y, n_ok, n_forced, n_tb, n_init, n_res, n_port, n_pin);
  process
  begin
    << signal .cleansig.dut.s_forced : std_ulogic_vector(3 downto 0) >> <= force "1UX1";
    << signal .cleansig.dut.s_tb : std_ulogic_vector(3 downto 0) >> <= "X1U1";
    for i in 1 to 4 loop
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
    end loop;
    report "ok=" & integer'image(n_ok) & " forced=" & integer'image(n_forced) & " tb="
      & integer'image(n_tb) & " init=" & integer'image(n_init) & " res=" & integer'image(n_res)
      & " port=" & integer'image(n_port) & " pin=" & integer'image(n_pin);
    wait;
  end process;
end architecture;
