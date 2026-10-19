-- difftest: top=force_composite_ports std=2008
-- The default force mode on a part of a composite port of mode out is `out` (LRM 10.5.2.1):
-- the driving value, which the actual sees. A forced `in` port is the discrepancy case
-- `force-in-port-actual`.
library ieee; use ieee.std_logic_1164.all;
entity fcp_sub is
  port (o : out std_logic_vector(3 downto 0); i : in std_logic_vector(3 downto 0));
end entity;
architecture a of fcp_sub is
begin
  o <= "0001";
  process begin
    wait for 1 ns;
    o(1 downto 0) <= force "10";
    wait for 1 ns;
    report "inside i=" & to_string(i) & " o=" & to_string(o);
    o <= release;
    wait;
  end process;
end architecture;
library ieee; use ieee.std_logic_1164.all;
entity force_composite_ports is end entity;
architecture a of force_composite_ports is
  signal so, si : std_logic_vector(3 downto 0) := "0000";
begin
  u : entity work.fcp_sub port map (o => so, i => si);
  si <= "0101";
  process begin
    wait for 0.5 ns;
    report "so=" & to_string(so) & " si=" & to_string(si);
    wait for 1 ns;
    report "forced so=" & to_string(so) & " si=" & to_string(si);
    assert so = "0010" severity failure;
    wait for 1 ns;
    report "released so=" & to_string(so);
    assert so = "0001" severity failure;
    wait;
  end process;
end architecture;
