-- 10 million xor operations on a 16-bit unsigned, against a signal (SIG true) or a variable (SIG
-- false): the operator-call cost in a tight loop.
-- Head-to-head design (tools/bench/AGENTS.md); written for yavs.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity bench_xor_sig is generic (N : natural := 10000000; SIG : boolean := true); end entity;
architecture a of bench_xor_sig is
  signal s : unsigned(15 downto 0) := x"1234";
begin
  process
    variable a, v : unsigned(15 downto 0) := x"00FF";
  begin
    for i in 1 to N loop
      if SIG then a := a xor s; else a := a xor v; end if;
    end loop;
    report integer'image(to_integer(a));
    wait;
  end process;
end architecture;
