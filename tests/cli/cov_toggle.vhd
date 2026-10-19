-- Toggle coverage counts (crates/yavs/tests/coverage.rs): every count below is pinned there.
library ieee;
use ieee.std_logic_1164.all;

entity cov_tog_sub is
  port (d : in std_logic; q : out std_logic);
end entity;

architecture rtl of cov_tog_sub is
begin
  q <= not d;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity cov_toggle is
end entity;

architecture sim of cov_toggle is
  type rec_t is record
    valid : std_logic;
    n : integer;
  end record;
  signal clk : bit := '0';
  signal s : std_logic := '0';
  signal v : std_logic_vector(1 downto 0) := "00";
  signal r : rec_t := ('0', 0);
  signal q : std_logic;
begin
  u1 : entity work.cov_tog_sub port map (d => s, q => q);
  process
  begin
    -- clk: 0->1 three times, 1->0 twice.
    for i in 1 to 3 loop
      clk <= '1';
      wait for 1 ns;
      if i < 3 then
        clk <= '0';
        wait for 1 ns;
      end if;
    end loop;
    -- s: '0' -> 'H' is a rise, 'H' -> 'Z' -> '0' no fall, then '0' -> '1' a rise.
    s <= 'H';
    wait for 1 ns;
    s <= 'Z';
    wait for 1 ns;
    s <= '0';
    wait for 1 ns;
    s <= '1';
    wait for 1 ns;
    -- v(0) rises and falls, v(1) never moves.
    v(0) <= '1';
    wait for 1 ns;
    v(0) <= '0';
    r.n <= 5;
    wait;
  end process;
end architecture;
