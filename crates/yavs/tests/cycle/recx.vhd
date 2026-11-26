-- #1203: under --two-state the metavalues of a record constant's literal take their --init-x
-- bits, as a vector constant's do, read whole, by field, by element in a loop or by a scalar field,
-- in both modes alike: a record and an array of records (cycle_recx), parts of a record next to a
-- vector constant (cycle_recx2).
library ieee;
use ieee.std_logic_1164.all;

entity cycle_recx_dut is
  port (clk : in std_ulogic; v : out std_ulogic_vector(15 downto 0));
end entity;

architecture rtl of cycle_recx_dut is
  type rec_t is record
    a : std_ulogic_vector(3 downto 0);
    s : std_ulogic;
  end record;
  type recs_t is array (0 to 1) of rec_t;
  constant r : rec_t := ("X1U0", 'U');
  constant t : recs_t := (("UUXX", 'X'), ("-1W0", 'Z'));
begin
  process (clk)
  begin
    if rising_edge(clk) then
      v <= r.a & r.s & t(0).a & t(0).s & t(1).a & t(1).s & "0";
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity cycle_recx is
end entity;

architecture tb of cycle_recx is
  signal clk : std_ulogic := '0';
  signal v : std_ulogic_vector(15 downto 0);
begin
  dut : entity work.cycle_recx_dut port map (clk, v);
  process
  begin
    clk <= '1';
    wait for 5 ns;
    report "v=" & to_string(v);
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity cycle_recx2_dut is
  port (clk : in std_ulogic; v : out std_ulogic_vector(11 downto 0));
end entity;

architecture rtl of cycle_recx2_dut is
  type rec_t is record
    a : std_ulogic_vector(3 downto 0);
    s : std_ulogic;
  end record;
  constant r : rec_t := ("X1U0", 'U');
  constant c : std_ulogic_vector(3 downto 0) := "X1U0";
begin
  process (clk)
    variable w : std_ulogic_vector(3 downto 0);
  begin
    if rising_edge(clk) then
      for i in 3 downto 0 loop
        w(i) := r.a(i);
      end loop;
      v <= r.a & w & r.s & c(3) & c(1) & r.a(3);
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity cycle_recx2 is
end entity;

architecture tb of cycle_recx2 is
  signal clk : std_ulogic := '0';
  signal v : std_ulogic_vector(11 downto 0);
begin
  dut : entity work.cycle_recx2_dut port map (clk, v);
  process
  begin
    clk <= '1';
    wait for 5 ns;
    report "v=" & to_string(v);
    wait;
  end process;
end architecture;
