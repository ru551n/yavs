-- difftest: top=null_waveform_unguarded_param std=2008 exit=1 ref=hand
-- A null waveform element on a signal parameter whose actual is not guarded (LRM 10.5.2.2).
-- ref=hand: the expected text is yavs's wording of the run-time error.
library ieee; use ieee.std_logic_1164.all;
entity null_waveform_unguarded_param is end entity;
architecture a of null_waveform_unguarded_param is
  signal u : std_logic;
  procedure d (signal s : out std_logic) is begin s <= '1', null after 2 ns; end procedure;
begin
  process begin d(u); wait; end process;
end architecture;
