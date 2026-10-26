-- One clocked register per generate iteration; the process line is replaced by each spelling of
-- the clock edge in turn (crates/yavs/tests/perf.rs, `edge_cost_parity`): every form costs the same.
library ieee; use ieee.std_logic_1164.all;
entity edge_cost is end;
architecture a of edge_cost is
  signal clk : std_logic := '0';
  signal q : std_logic_vector(7 downto 0) := (others => '0');
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  g: for i in 0 to 7 generate
    process (clk) begin if rising_edge(clk) then q(i) <= not q(i); end if; end process;
  end generate;
  process
    variable ones : natural := 0;
  begin
    wait for 1000 ns;
    done <= true;
    wait for 10 ns;
    for i in q'range loop if q(i) = '1' then ones := ones + 1; end if; end loop;
    report "ones=" & integer'image(ones);
    wait;
  end process;
end;
