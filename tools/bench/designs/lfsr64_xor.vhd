-- 64 16-bit LFSRs (one process each, numeric_std unsigned) feeding a 64-way XOR reduction every
-- cycle, 5000 cycles: process wake-up plus a reduction loop over an array signal.
-- Head-to-head design (tools/bench/AGENTS.md); written for yavs.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity bench_lfsr64_xor is
  generic (NREG : natural := 64; CYCLES : natural := 5000);
end entity;
architecture a of bench_lfsr64_xor is
  type word_arr is array (0 to NREG - 1) of unsigned(15 downto 0);
  signal clk : std_logic := '0';
  signal q : word_arr;
  signal sums : unsigned(15 downto 0) := (others => '0');
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done else clk;
  g : for i in 0 to NREG - 1 generate
    process (clk)
      variable v : unsigned(15 downto 0) := to_unsigned(i * 37 + 1, 16);
    begin
      if rising_edge(clk) then
        v := v(14 downto 0) & (v(15) xor v(13) xor v(12) xor v(10));
        q(i) <= v;
      end if;
    end process;
  end generate;
  process (clk)
    variable a : unsigned(15 downto 0);
    variable n : natural := 0;
  begin
    if rising_edge(clk) then
      a := (others => '0');
      for i in 0 to NREG - 1 loop a := a xor q(i); end loop;
      sums <= a;
      n := n + 1;
      if n = CYCLES then
        done <= true;
        report "sum=" & integer'image(to_integer(a));
      end if;
    end if;
  end process;
end architecture;
