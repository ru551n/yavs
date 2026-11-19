-- Cycle mode: a register outside the region on a clock copied twice (a delta after the clock of
-- the member register that feeds it through combinational logic) samples that logic before the
-- normal mode's new value arrives. It stays out only for a later rule (an output port associated in
-- parts), so the run-count cone is computed again then. Found by the fuzzer
-- (tools/difftest/cyclefuzz.py); the logic had stayed in the region and the register sampled it settled.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_latecone_reg is
  generic (W : positive := 8);
  port (clk : in std_ulogic; s : in std_ulogic; a : in unsigned(W - 1 downto 0);
        y : out unsigned(W - 1 downto 0) := (others => '0'); z : out std_ulogic_vector(1 downto 0) := "00");
end entity;
architecture a of cycle_latecone_reg is
  signal c : unsigned(W - 1 downto 0) := (others => '0');
begin
  c <= a + 1 when s = '1' else not a;
  process (clk)
  begin
    if rising_edge(clk) then
      y <= c;
      z <= s & c(0);
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_latecone is
  generic (W : positive := 8; G : boolean := true);
end entity;
architecture a of cycle_latecone is
  signal clk, dk1, dk2, s : std_ulogic := '0';
  signal cnt : unsigned(W - 1 downto 0) := (others => '0');
  signal y : unsigned(W - 1 downto 0);
  signal za, zb : std_ulogic;
begin
  clk <= not clk after 5 ns;
  dk1 <= clk;
  dk2 <= dk1 when G else '0';
  first : process (dk1)
  begin
    if rising_edge(dk1) then
      cnt <= cnt + 3;
      s <= not s;
    end if;
  end process;
  u : entity work.cycle_latecone_reg
    generic map (W => W)
    port map (clk => dk2, s => s, a => cnt, y => y, z(0) => za, z(1) => zb);
  stim : process
  begin
    for i in 0 to 39 loop
      wait until falling_edge(clk);
      report "cnt=" & to_hstring(cnt) & " y=" & to_hstring(y) & " z=" & std_ulogic'image(zb) & std_ulogic'image(za);
    end loop;
    std.env.finish;
  end process;
end architecture;
