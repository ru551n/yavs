-- An input of the region that nothing drives keeps its start value 'U' for the whole run: the
-- normal mode takes the else branch of `v = '0'` (q = '1'), the region reads 0 (q = '0'). Cycle
-- mode names such inputs at the start (a run that differs this way may never end). With
-- `drive` the signal has a driver and nothing is said.
library ieee; use ieee.std_logic_1164.all;
entity cycle_undriven is
  generic (drive : boolean := false);
end;
architecture a of cycle_undriven is
  signal clk : std_ulogic := '0';
  signal v : std_logic;
  signal w : std_logic_vector(3 downto 0) := "0000";
  signal q : std_logic := '0';
begin
  clk <= not clk after 5 ns when now < 30 ns else clk;
  g : if drive generate
    v <= '1';
  end generate;
  m : process (clk) begin
    if rising_edge(clk) then
      if v = '0' or w(0) = '1' then q <= '0'; else q <= '1'; end if;
    end if;
  end process;
  chk : process begin
    wait for 40 ns;
    report "q=" & std_logic'image(q);
    wait;
  end process;
end;
