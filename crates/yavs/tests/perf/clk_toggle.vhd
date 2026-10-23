-- One clock generator and one process that toggles a flop on the rising edge, 100 cycles:
-- the clock runs in the kernel (no process), so only the flop's process is activated.
library ieee; use ieee.std_logic_1164.all;
entity clk_toggle is end;
architecture a of clk_toggle is
  signal clk : std_logic := '0';
  signal q : std_logic := '0';
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  process (clk) begin
    if rising_edge(clk) then q <= not q; end if;
  end process;
  process begin
    wait for 1 us;
    report "q=" & std_logic'image(q);
    done <= true; wait;
  end process;
end;
