-- difftest: top=disconnect_time std=2008
-- A disconnection specification delays the disconnection of a guarded signal's driver (LRM 7.4).
library ieee; use ieee.std_logic_1164.all;
entity disconnect_time is end;
architecture t of disconnect_time is
  signal en : boolean := false;
  signal slow, fast : std_logic bus;
  disconnect slow : std_logic after 3 ns;
begin
  g : block (en) begin
    slow <= guarded '1';
    fast <= guarded '1';
  end block;
  process begin
    en <= true; wait for 1 ns;
    report "on " & std_logic'image(slow) & std_logic'image(fast);
    en <= false; wait for 1 ns;
    report "off 1 ns " & std_logic'image(slow) & std_logic'image(fast);
    wait for 1 ns;
    report "off 2 ns " & std_logic'image(slow) & std_logic'image(fast);
    wait for 1 ns;
    report "off 3 ns " & std_logic'image(slow) & std_logic'image(fast);
    wait;
  end process;
end;
