-- A foreign architecture implemented by the VHPI plugin tests/plugin/farch.c (#330): an
-- inverter with a delay and an event counter, driven only through vhpi_schedule_transaction.
library ieee;
use ieee.std_logic_1164.all;

entity inv is
  generic (delay_ns : natural := 2);
  port (
    a : in  std_logic_vector(3 downto 0);
    y : out std_logic_vector(3 downto 0);
    n : out integer := -1
  );
end entity;

architecture model of inv is
  attribute foreign of model : architecture is "VHPI plug inv";
begin
  -- Not elaborated: the model stands in for the architecture.
  y <= "ZZZZ";
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity top is end;
architecture a of top is
  signal a : std_logic_vector(3 downto 0) := "0000";
  signal y : std_logic_vector(3 downto 0);
  signal n : integer;
begin
  u : entity work.inv port map (a => a, y => y, n => n);
  process
  begin
    wait for 0 ns;
    assert y = "1111" and n = -1 report "initial " & to_string(y) severity failure;
    a <= "0011";
    wait for 1 ns;
    assert y = "1111" report "before the delay" severity failure;
    wait for 1 ns;
    assert y = "1100" report "after the delay " & to_string(y) severity failure;
    wait for 3 ns;
    assert n = 1 report "transport count" severity failure;
    -- A pulse shorter than the inertial delay is rejected.
    a <= "0101";
    wait for 1 ns;
    a <= "0011";
    wait for 5 ns;
    assert y = "1100" and y'last_event > 5 ns report "pulse not rejected" severity failure;
    assert n = 3 report "count " & integer'image(n) severity failure;
    report "farch ok";
    wait;
  end process;
end architecture;

-- Two instances: one process and driver set each.
library ieee;
use ieee.std_logic_1164.all;

entity two is end;
architecture a of two is
  signal a : std_logic_vector(3 downto 0) := "0000";
  signal y1, y2 : std_logic_vector(3 downto 0);
  signal n1, n2 : integer;
begin
  u1 : entity work.inv port map (a => a, y => y1, n => n1);
  u2 : entity work.inv generic map (delay_ns => 1) port map (a => y1, y => y2, n => n2);
  process
  begin
    wait for 1 ns;
    a <= "1010";
    wait for 10 ns;
    assert y1 = "0101" and y2 = "1010" report "chain " & to_string(y1) & to_string(y2)
      severity failure;
    report "two ok";
    wait;
  end process;
end architecture;

-- An architecture whose model no plugin registered.
entity lost_inv is
  port (b : out bit);
end entity;
architecture model of lost_inv is
  attribute foreign of model : architecture is "VHPI plug lost";
begin
end architecture;

entity lost is end;
architecture a of lost is
  signal b : bit;
begin
  u : entity work.lost_inv port map (b => b);
end architecture;
