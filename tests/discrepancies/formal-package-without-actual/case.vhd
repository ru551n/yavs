library ieee; use ieee.std_logic_1164.all;
package slv is
  generic (N : integer);
  subtype slv_t is std_logic_vector(N - 1 downto 0);
end package;

library ieee; use ieee.std_logic_1164.all;
entity ent is
  generic (WIDTH : integer; package slv_pkg is new work.slv generic map (N => WIDTH));
  port (o_slv : out slv_pkg.slv_t);
end ent;
architecture beh of ent is begin o_slv <= (others => '1'); end architecture;

library ieee; use ieee.std_logic_1164.all;
entity top is end entity;
architecture a of top is
  signal s : std_logic_vector(3 downto 0);
begin
  e : entity work.ent generic map (WIDTH => 4) port map (o_slv => s);
  process begin wait for 1 ns; report "RES " & to_string(s); wait; end process;
end architecture;
