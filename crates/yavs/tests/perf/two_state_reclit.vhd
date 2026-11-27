-- `--two-state`: a record constant with metavalues read at every edge (a register's default, an
-- element of a table of records) maps each value once, not at every read.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity two_state_reclit is
end entity;
architecture a of two_state_reclit is
  type reg_t is record
    a : std_ulogic_vector(7 downto 0);
    b : std_ulogic;
  end record;
  constant INIT : reg_t := (a => "0101--XU", b => '-');
  type tab_t is array (0 to 3) of reg_t;
  constant TAB : tab_t := (("00001111", '0'), ("1111----", '1'), ("XXXX0000", 'U'), ("01010101", '1'));
  signal clk : std_ulogic := '0';
  signal n : natural := 0;
begin
  process (clk)
    variable r : reg_t;
    variable acc : natural := 0;
  begin
    if rising_edge(clk) then
      r := INIT;
      if r.a(7 downto 4) = "0101" then acc := acc + 1; end if;
      r := TAB(n mod 4);
      if r.b = '1' then acc := acc + 3; end if;
      n <= acc;
    end if;
  end process;
  process
  begin
    for i in 1 to 300 loop
      clk <= '1'; wait for 1 ns; clk <= '0'; wait for 1 ns;
    end loop;
    report "acc=" & integer'image(n);
    wait;
  end process;
end architecture;
