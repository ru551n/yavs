-- 4000 processes sensitive to one clock that do almost nothing (enable low), 5000 cycles: wake-up
-- and suspend cost per process.
-- Head-to-head design (tools/bench/AGENTS.md); written for yavs.
library ieee; use ieee.std_logic_1164.all;
entity bench_rtl_fan is generic (N : natural := 4000; CYC : natural := 5000); end;
architecture a of bench_rtl_fan is
  signal clk : std_logic := '0';
  signal en : std_logic := '0';
  signal done : boolean := false;
  signal hit : natural := 0;
begin
  clk <= not clk after 5 ns when not done;
  g: for i in 0 to N-1 generate
    process (clk) variable cnt : natural := 0; begin
      if rising_edge(clk) and en = '1' then cnt := cnt + 1; end if;
    end process;
  end generate;
  process begin
    for c in 1 to CYC loop wait until rising_edge(clk); end loop;
    report "done";
    done <= true; wait;
  end process;
end;
