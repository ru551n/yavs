-- 32 drivers on one resolved std_logic_vector bus, one driving per cycle and 31 releasing ('Z'),
-- 20000 cycles: resolution function and driver bookkeeping.
-- Head-to-head design (tools/bench/AGENTS.md); written for yavs.
library ieee; use ieee.std_logic_1164.all;
entity bench_rtl_bus is generic (D : natural := 32; CYC : natural := 20000); end;
architecture a of bench_rtl_bus is
  signal clk : std_logic := '0';
  signal b : std_logic_vector(31 downto 0);
  signal sel : natural range 0 to D-1 := 0;
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  g: for i in 0 to D-1 generate
    process (clk) begin
      if rising_edge(clk) then
        if sel = i then b <= (others => '0'); b(i) <= '1'; else b <= (others => 'Z'); end if;
      end if;
    end process;
  end generate;
  process
    variable ones : natural := 0;
  begin
    for c in 1 to CYC loop
      wait until rising_edge(clk);
      sel <= (sel + 1) mod D;
      for i in b'range loop if b(i) = '1' then ones := ones + 1; end if; end loop;
    end loop;
    report "ones=" & integer'image(ones);
    done <= true; wait;
  end process;
end;
