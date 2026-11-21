-- A guarded combinational member under --cycle-exact-meta whose choice reads the region's image
-- (`s = '1'`): `s` goes from 'U' to '1', which the second image already read as 1, so only the
-- region's image changed. The second image must run the member again, not skip it on its own
-- unchanged inputs (the test runs with YAVS_CYCLE_GUARD=all).
library ieee; use ieee.std_logic_1164.all;
entity cycle_railguard is
  generic (W : positive := 8);
end;
architecture a of cycle_railguard is
  signal clk : std_ulogic := '0';
  signal s : std_ulogic;
  signal a : std_ulogic_vector(W - 1 downto 0) := (others => '1');
  signal b : std_ulogic_vector(W - 1 downto 0) := (others => '0');
  signal y, q : std_ulogic_vector(W - 1 downto 0);
begin
  clk <= not clk after 5 ns when now < 100 ns else clk;
  y <= a when s = '1' else b;
  ck : process (clk) begin
    if rising_edge(clk) then
      q <= y;
    end if;
  end process;
  stim : process begin
    wait for 12 ns;
    report "y=" & to_string(y);
    s <= '1';
    wait for 20 ns;
    report "y=" & to_string(y) & " q=" & to_string(q);
    wait;
  end process;
end;
