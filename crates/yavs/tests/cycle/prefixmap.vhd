-- --two-state: an element, slice or field of a constant holding metavalues reads the bits its
-- literal took (--init-x), whether the constant is the architecture's or the process's, and a
-- record signal's field read alone maps like the whole.
library ieee;
use ieee.std_logic_1164.all;

entity prefixmap_dut is
  port (clk : in std_ulogic; n : out natural);
end entity;

architecture rtl of prefixmap_dut is
  type rec_t is record
    v : std_ulogic_vector(3 downto 0);
    s : std_ulogic;
  end record;
  constant ac : std_ulogic_vector(3 downto 0) := "U1X0";
  constant ar : rec_t := ("X1U0", 'U');
  signal rs : rec_t := ("H1UL", 'H');
begin
  process (clk)
    constant pc : std_ulogic_vector(3 downto 0) := "U1X0";
    variable c : natural := 0;
  begin
    if rising_edge(clk) then
      for i in 3 downto 0 loop
        if ac(i) = '1' then c := c + 1; end if;
        if pc(i) = '1' then c := c + 10; end if;
        if ar.v(i) = '1' then c := c + 100; end if;
        if rs.v(i) = '1' then c := c + 1000; end if;
      end loop;
      if ac(3 downto 2) = "11" then c := c + 10000; end if;
      if ar.s = '1' then c := c + 100000; end if;
      if rs.s = '1' then c := c + 1000000; end if;
      n <= c;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity prefixmap is
end entity;

architecture tb of prefixmap is
  signal clk : std_ulogic := '0';
  signal n : natural;
begin
  dut : entity work.prefixmap_dut port map (clk, n);
  process
  begin
    clk <= '1';
    wait for 5 ns;
    clk <= '0';
    wait for 5 ns;
    report "n=" & integer'image(n);
    wait;
  end process;
end architecture;
