-- A 16-stage shift register, one process per stage, each driving one element of one vector:
-- vector direct entries of one signal committed as a run, one wake per stage and edge.
library ieee; use ieee.std_logic_1164.all;
entity shift_chain is end;
architecture a of shift_chain is
  signal clk : std_logic := '0';
  signal s : std_logic_vector(0 to 16) := (others => '0');
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  process (clk) begin
    if rising_edge(clk) then s(0) <= not s(0); end if;
  end process;
  g: for i in 1 to 16 generate
    process (clk) begin
      if rising_edge(clk) then s(i) <= s(i-1); end if;
    end process;
  end generate;
  process begin
    wait for 1 us;
    report "s=" & to_string(s);
    done <= true; wait;
  end process;
end;
