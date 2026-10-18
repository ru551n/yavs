-- difftest: top=force_drivers std=2008 stop=60ns
library ieee; use ieee.std_logic_1164.all;
entity force_drivers is end;
architecture t of force_drivers is
  signal s : std_logic := '0';
  signal clk : bit := '0';
begin
  clk <= not clk after 5 ns;
  s <= '1' after 12 ns, '0' after 22 ns;
  process begin
    wait for 8 ns; s <= force '1';
    wait for 10 ns; s <= release;
    wait for 15 ns; s <= force 'X'; wait for 3 ns; s <= release;
    wait;
  end process;
  process (s) begin report "s=" & std_logic'image(s) & " @" & time'image(now); end process;
end;
