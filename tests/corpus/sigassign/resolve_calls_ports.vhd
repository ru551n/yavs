-- difftest: top=resolve_calls_ports
-- A resolution function that reports each call, on an integer signal driven through the out
-- ports of three instances and by a process: once per update with all four sources (LRM
-- 14.7.3.2), not once per port.
library ieee; use ieee.std_logic_1164.all;
package resolve_calls_ports_pkg is
  type int_v is array (natural range <>) of integer;
  function res_sum (v : int_v) return integer;
  subtype rint is res_sum integer;
end;
package body resolve_calls_ports_pkg is
  function res_sum (v : int_v) return integer is
    variable s : integer := 0;
  begin
    for i in v'range loop s := s + v(i); end loop;
    if s /= 0 then
      report "res_sum of " & integer'image(v'length) & " sources: " & integer'image(s);
    end if;
    return s;
  end;
end;
library ieee; use ieee.std_logic_1164.all;
entity resolve_calls_ports_one is
  port (clk : in std_logic; k : in integer; o : out integer := 0);
end;
architecture a of resolve_calls_ports_one is
begin
  process (clk)
    variable n : integer := 0;
  begin
    if rising_edge(clk) then n := n + 1; o <= n * k; end if;
  end process;
end;
library ieee; use ieee.std_logic_1164.all; use work.resolve_calls_ports_pkg.all;
entity resolve_calls_ports is end;
architecture a of resolve_calls_ports is
  signal clk : std_logic := '0';
  signal r : rint := 0;
begin
  process begin
    for i in 1 to 3 loop clk <= '1'; wait for 5 ns; clk <= '0'; wait for 5 ns; end loop; wait;
  end process;
  g : for k in 1 to 3 generate
    u : entity work.resolve_calls_ports_one port map (clk => clk, k => k, o => r);
  end generate;
  process (clk)
    variable n : integer := 0;
  begin
    if rising_edge(clk) then n := n + 1; r <= n * 100; end if;
  end process;
  process (r) begin report "r=" & integer'image(r); end process;
end;
