-- difftest: top=bus_guarded std=2008 ref=hand
-- ref=hand: sim1 gets this wrong (see README); .expected is hand-corrected to the LRM result.
library ieee; use ieee.std_logic_1164.all;
entity bus_guarded is end;
architecture t of bus_guarded is
  signal en1, en2 : boolean := false;
  signal b : std_logic bus;
begin
  g1 : block (en1) begin b <= guarded '1'; end block;
  g2 : block (en2) begin b <= guarded '0'; end block;
  process begin
    report "idle " & std_logic'image(b);
    en1 <= true; wait for 1 ns; report "one " & std_logic'image(b);
    en1 <= false; wait for 1 ns; report "released " & std_logic'image(b);
    en2 <= true; wait for 1 ns; report "two " & std_logic'image(b);
    en1 <= true; wait for 1 ns; report "conflict " & std_logic'image(b);
    wait;
  end process;
end;
