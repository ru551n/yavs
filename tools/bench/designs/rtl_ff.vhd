-- 4096 one-bit registers, one process each (clocked, edge detect), a nor/xor next-state function
-- and 2000 cycles: many tiny processes waking on one clock, scalar signal updates.
-- Head-to-head design (tools/bench/AGENTS.md); written for yavs.
library ieee; use ieee.std_logic_1164.all;
entity bench_rtl_ff is generic (N : natural := 4096; CYC : natural := 2000); end;
architecture a of bench_rtl_ff is
  signal clk : std_logic := '0';
  signal q : std_logic_vector(N-1 downto 0) := (others => '0');
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  g: for i in 0 to N-1 generate
    process (clk) begin
      if rising_edge(clk) then
        q(i) <= q((i+1) mod N) xor q((i+7) mod N) xor (q(i) nor q((i+3) mod N));
      end if;
    end process;
  end generate;
  process
    variable ones : natural := 0;
  begin
    for c in 1 to CYC loop wait until rising_edge(clk); end loop;
    for i in q'range loop if q(i) = '1' then ones := ones + 1; end if; end loop;
    report "ones=" & integer'image(ones);
    done <= true; wait;
  end process;
end;
