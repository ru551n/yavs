-- #1201, #1202: std_match and ?= against a field of a record-constant table keep its '-' in every
-- mode (loop index, index chosen at run time, constant index); a call on a '-' constant is not
-- folded where two-state code maps the argument first.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity cycle_dashtab_dut is
  port (
    clk : in std_ulogic;
    a, b, x : in std_ulogic_vector(3 downto 0);
    y : out std_ulogic_vector(3 downto 0);
    n1, n2, n3 : out natural);
end entity;

architecture rtl of cycle_dashtab_dut is
  type rec_t is record
    pat : std_ulogic_vector(3 downto 0);
    id : natural;
  end record;
  type tab_t is array (0 to 1) of rec_t;
  constant tab : tab_t := (("--11", 1), ("0-1-", 2));
  constant pc : std_ulogic_vector(3 downto 0) := "--11";
  function zeros(p : std_ulogic_vector) return natural is
    variable n : natural := 0;
  begin
    for i in p'range loop
      if p(i) = '0' then
        n := n + 1;
      end if;
    end loop;
    return n;
  end function;
begin
  y <= a when zeros(pc) = 0 else b;
  by_loop : process (clk)
    variable c : natural := 0;
  begin
    if rising_edge(clk) then
      for i in tab'range loop
        if std_match(x, tab(i).pat) then
          c := c + 1 + i;
        end if;
      end loop;
      n1 <= c;
    end if;
  end process;
  by_pick : process (clk)
    variable c : natural := 0;
  begin
    if rising_edge(clk) then
      if std_match(x, tab(to_integer(unsigned(x(0 downto 0)))).pat) then
        c := c + 1;
      end if;
      n2 <= c;
    end if;
  end process;
  by_const : process (clk)
    variable c : natural := 0;
  begin
    if rising_edge(clk) then
      if (x ?= tab(1).pat) = '1' then
        c := c + 1;
      end if;
      if x(3 downto 2) ?= tab(0).pat(3 downto 2) then
        c := c + 100;
      end if;
      n3 <= c;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity cycle_dashtab is
end entity;

architecture tb of cycle_dashtab is
  signal clk : std_ulogic := '0';
  signal a, b, x, y : std_ulogic_vector(3 downto 0) := "0000";
  signal n1, n2, n3 : natural;
  signal ys : natural := 0;
begin
  dut : entity work.cycle_dashtab_dut port map (clk, a, b, x, y, n1, n2, n3);
  main : process
  begin
    for i in 0 to 15 loop
      x <= std_ulogic_vector(to_unsigned(i, 4));
      a <= std_ulogic_vector(to_unsigned(i, 4));
      b <= std_ulogic_vector(to_unsigned(i mod 3, 4));
      wait for 1 ns;
      ys <= ys + to_integer(unsigned(y));
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 4 ns;
    end loop;
    report "n1=" & integer'image(n1) & " n2=" & integer'image(n2) & " n3=" & integer'image(n3)
      & " ys=" & integer'image(ys);
    wait;
  end process;
end architecture;
