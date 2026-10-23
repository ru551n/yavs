-- difftest: top=others_unconstrained_target std=2008 relaxed
-- Accepted with the relaxed switch: an others aggregate assigned to a port, a signal formal
-- and a variable formal of unconstrained type takes the bounds the object has (the actual's),
-- ascending and descending.
library ieee;
use ieee.std_logic_1164.all;

entity others_unconstrained_target_c is
  port (o : out std_logic_vector; o2 : inout std_logic_vector);
end entity;

architecture a of others_unconstrained_target_c is
  procedure clr (variable v : out std_logic_vector) is
  begin
    v := (others => '0');
  end procedure;
  procedure hiz (signal v : out std_logic_vector) is
  begin
    v <= (others => 'Z');
  end procedure;
begin
  o <= (others => '1');
  process
    variable x : std_logic_vector(5 downto 2) := "1111";
  begin
    clr(x);
    assert x = "0000" report "variable formal" severity failure;
    assert x'left = 5 and x'right = 2 report "variable bounds" severity failure;
    wait for 1 ns;
    hiz(o2);
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity others_unconstrained_target is end entity;

architecture t of others_unconstrained_target is
  signal s1 : std_logic_vector(3 downto 0);
  signal s2 : std_logic_vector(1 to 3) := "101";
begin
  u : entity work.others_unconstrained_target_c port map (s1, s2);
  process
  begin
    wait for 2 ns;
    assert s1 = "1111" report "port" severity failure;
    assert s2 = "ZZZ" report "signal formal" severity failure;
    report "ok " & to_string(s1) & " " & to_string(s2);
    wait;
  end process;
end architecture;
