-- 512 16-bit LFSRs in an array signal, one process each, 5000 cycles: slice and concatenation of
-- std_logic_vector, element-wise signal assignment.
-- Head-to-head design (tools/bench/AGENTS.md); written for yavs.
library ieee; use ieee.std_logic_1164.all;
entity bench_rtl_lfsr is generic (N : natural := 512; CYC : natural := 5000); end;
architecture a of bench_rtl_lfsr is
  signal clk : std_logic := '0';
  type arr is array (natural range <>) of std_logic_vector(15 downto 0);
  signal r : arr(0 to N-1);
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  g: for i in 0 to N-1 generate
    process (clk) begin
      if rising_edge(clk) then
        if r(i)(0) = 'U' then
          r(i) <= x"ACE1";
        else
          r(i) <= r(i)(14 downto 0) & (r(i)(15) xor r(i)(13) xor r(i)(12) xor r(i)(10));
        end if;
      end if;
    end process;
  end generate;
  process
    variable acc : std_logic_vector(15 downto 0) := (others => '0');
  begin
    for c in 1 to CYC loop wait until rising_edge(clk); end loop;
    for i in r'range loop acc := acc xor r(i); end loop;
    report "acc=" & to_string(acc);
    done <= true; wait;
  end process;
end;
