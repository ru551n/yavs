-- `--two-state --init-x=1`: constants with metavalues read whole at every edge (a record, a vector,
-- a table of records) map once per run, and a `(others => '0')` of a run-time length maps nothing:
-- no `yavs.two.lit` call at the reads.
library ieee;
use ieee.std_logic_1164.all;
entity two_state_litonce is
end entity;
architecture a of two_state_litonce is
  type reg_t is record
    a : std_ulogic_vector(7 downto 0);
    b : std_ulogic;
  end record;
  type tab_t is array (0 to 1) of reg_t;
  constant INIT : reg_t := (a => "0101--XU", b => '-');
  constant C : std_ulogic_vector(7 downto 0) := "X1U0-ZWL";
  constant TAB : tab_t := (("00001111", 'U'), ("1111XXXX", '1'));
  signal clk : std_ulogic := '0';
  signal n : natural := 0;
begin
  process (clk)
    variable r : reg_t;
    variable t : tab_t;
    variable w : std_ulogic_vector(15 downto 0);
    variable acc : natural := 0;
  begin
    if rising_edge(clk) then
      r := INIT;
      t := TAB;
      w := C & C;
      w(n mod 8 downto 0) := (others => '0');
      if r.a(7 downto 4) = "0101" then acc := acc + 1; end if;
      if t(1).a(3) = '1' then acc := acc + 2; end if;
      if w(15) = '1' then acc := acc + 4; end if;
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
