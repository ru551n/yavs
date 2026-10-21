-- Tight loop of one operator on 16-bit vectors, selected by generic K: 0 unsigned xor, 1
-- std_logic_vector xor, 2 bit_vector xor, 3 unsigned +.
-- Head-to-head design (tools/bench/AGENTS.md); written for yavs.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity bench_xor_types is generic (N : natural := 10000000; K : natural := 0); end entity;
architecture a of bench_xor_types is
begin
  process
    variable u, w : unsigned(15 downto 0) := x"00FF";
    variable l, m : std_logic_vector(15 downto 0) := x"00FF";
    variable b, c : bit_vector(15 downto 0) := x"00FF";
  begin
    for i in 1 to N loop
      case K is
        when 0 => u := u xor w;
        when 1 => l := l xor m;
        when 2 => b := b xor c;
        when 3 => u := u + w;
        when others => null;
      end case;
    end loop;
    report integer'image(to_integer(u));
    wait;
  end process;
end architecture;
