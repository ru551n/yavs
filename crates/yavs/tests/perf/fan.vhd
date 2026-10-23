-- 32 processes sensitive to one clock with an enable that stays low, 50 cycles: a process whose
-- edge is not true is not run (edge trigger), only the enabled one counts.
library ieee; use ieee.std_logic_1164.all;
entity fan is end;
architecture a of fan is
  signal clk : std_logic := '0';
  signal en : std_logic := '0';
  signal done : boolean := false;
  signal hits : natural := 0;
begin
  clk <= not clk after 5 ns when not done;
  g: for i in 0 to 31 generate
    process (clk) variable cnt : natural := 0; begin
      if rising_edge(clk) and en = '1' then cnt := cnt + 1; end if;
    end process;
  end generate;
  process (clk) begin
    if rising_edge(clk) then hits <= hits + 1; end if;
  end process;
  process begin
    wait for 500 ns;
    report "hits=" & integer'image(hits);
    done <= true; wait;
  end process;
end;
