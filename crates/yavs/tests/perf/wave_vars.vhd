-- Processes with variables recorded with --wave-variables: one runs every cycle, one only when
-- its enable pulses; the idle one is asked once, not per variable.
library ieee; use ieee.std_logic_1164.all;
entity wave_vars is end;
architecture a of wave_vars is
  signal clk : std_logic := '0';
  signal en : std_logic := '0';
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  process (clk)
    variable a, b, c : natural := 0;
  begin
    if rising_edge(clk) then a := a + 1; b := b + 2; c := c + 3; end if;
  end process;
  process (clk)
    variable p, q, r, s : natural := 0;
  begin
    if rising_edge(clk) and en = '1' then p := p + 1; q := q + 2; r := r + 3; s := s + 4; end if;
  end process;
  process begin
    wait for 105 ns; en <= '1'; wait for 10 ns; en <= '0';
    wait for 385 ns;
    report "done";
    done <= true; wait;
  end process;
end;
