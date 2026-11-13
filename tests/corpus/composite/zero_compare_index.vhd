-- difftest: top=zero_compare_index std=2008 exit=1 ignore=fatalmsg
-- ignore=fatalmsg: the references word the index error differently (so did yavs before this test).
-- An element scan whose index leaves the memory: the fill's bounds come from `mem(i)'range`
-- without reading the element, and the scan still fails at the element past the end.
library ieee;
use ieee.std_logic_1164.all;

entity zero_compare_index is
  generic (R : positive := 4; W : positive := 8);
end entity;

architecture tb of zero_compare_index is
  type slv_array_t is array (natural range <>) of std_logic_vector;
  signal mem : slv_array_t(0 to R - 1)(W - 1 downto 0) := (others => (others => '0'));
begin
  process
    variable n : natural := 0;
  begin
    wait for 2 ns;
    for i in 0 to R loop
      if (mem(i)'range => '0') /= mem(i) then n := n + 100; end if;
      n := n + 1;
      report "checked " & integer'image(n);
    end loop;
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
