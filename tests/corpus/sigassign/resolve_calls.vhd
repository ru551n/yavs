-- difftest: top=resolve_calls
-- A resolution function that reports each call, on an integer signal with three drivers that
-- all assign every clock: it is called once per update of the signal with all its sources
-- (LRM 14.7.3.2), not once per driver whose transaction came in.
library ieee; use ieee.std_logic_1164.all;
package resolve_calls_pkg is
  type int_v is array (natural range <>) of integer;
  function res_max (v : int_v) return integer;
  subtype rint is res_max integer;
end;
package body resolve_calls_pkg is
  function res_max (v : int_v) return integer is
    variable m : integer := integer'low;
  begin
    for i in v'range loop if v(i) > m then m := v(i); end if; end loop;
    -- (Not at initialisation, where the reference simulators trace the call on stderr.)
    if m /= 0 then
      report "res_max of " & integer'image(v'length) & " sources: " & integer'image(m);
    end if;
    return m;
  end;
end;
library ieee; use ieee.std_logic_1164.all; use work.resolve_calls_pkg.all;
entity resolve_calls is end;
architecture a of resolve_calls is
  signal clk : std_logic := '0';
  signal r : rint := 0;
begin
  process begin
    for i in 1 to 4 loop clk <= '1'; wait for 5 ns; clk <= '0'; wait for 5 ns; end loop; wait;
  end process;
  g : for k in 0 to 2 generate
    process (clk)
      variable n : integer := 0;
    begin
      if rising_edge(clk) then
        n := n + 1;
        if k = 1 then r <= n * 10; else r <= k; end if;
      end if;
    end process;
  end generate;
  process (r) begin report "r=" & integer'image(r); end process;
end;
