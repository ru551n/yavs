-- Every spelling of a clock edge costs the same (#632): rising_edge, falling_edge, 'event with
-- a level either way round, not 'stable (no implicit signal), the predefined edges of BIT and
-- BOOLEAN, 'event and a bare BOOLEAN, and the wait forms. Each counts its edges in n.
library ieee; use ieee.std_logic_1164.all;
entity edge_forms is end;
architecture a of edge_forms is
  signal clk : std_logic := '0';
  signal cb : bit := '0';
  signal cbool : boolean := false;
  type cnt_t is array (1 to 12) of natural;
  signal n : cnt_t := (others => 0);
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  cb <= not cb after 5 ns when not done;
  cbool <= not cbool after 5 ns when not done;
  process (clk) begin if rising_edge(clk) then n(1) <= n(1) + 1; end if; end process;
  process (clk) begin if falling_edge(clk) then n(2) <= n(2) + 1; end if; end process;
  process (clk) begin if clk'event and clk = '1' then n(3) <= n(3) + 1; end if; end process;
  process (clk) begin if clk = '0' and clk'event then n(4) <= n(4) + 1; end if; end process;
  process (clk) begin if not clk'stable and clk = '1' then n(5) <= n(5) + 1; end if; end process;
  process (cb) begin if rising_edge(cb) then n(6) <= n(6) + 1; end if; end process;
  process (cb) begin if cb'event and cb = '0' then n(7) <= n(7) + 1; end if; end process;
  process (cbool) begin if rising_edge(cbool) then n(8) <= n(8) + 1; end if; end process;
  process (cbool) begin if cbool'event and cbool then n(9) <= n(9) + 1; end if; end process;
  process begin wait until clk = '1'; n(10) <= n(10) + 1; end process;
  process begin wait until rising_edge(clk); n(11) <= n(11) + 1; end process;
  process begin wait until clk'event and clk = '0'; n(12) <= n(12) + 1; end process;
  process
    variable sum : natural := 0;
  begin
    wait for 1000 ns;
    done <= true;
    wait for 10 ns;
    for i in n'range loop sum := sum + n(i); end loop;
    report "edges=" & integer'image(sum);
    wait;
  end process;
end;
