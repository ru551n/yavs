-- Asserts and reports under `--cycle-exact-meta`: `mon.process` (below the top) reports a register
-- with a 'U' start and reads the testbench's `d`, so it stays in the event kernel and reports the
-- 'U' as the normal mode does; under the default contract it joins and reports 0. `chk` (the
-- top-level unit) reads only a member's register: it joins and reports the 'U' start as 'U' under
-- `--cycle-exact-meta` (from the second image), as 0 under the default contract.
library ieee;
use ieee.std_logic_1164.all;

entity cycle_exactnote_mon is
  port (clk : in std_logic; d : in std_logic);
end;

architecture a of cycle_exactnote_mon is
  signal q, r : std_logic;
begin
  process (clk)
  begin
    if rising_edge(clk) then
      q <= d;
      report "q=" & to_string(q) & " r=" & to_string(r);
    end if;
  end process;
  r <= q;
end;

library ieee;
use ieee.std_logic_1164.all;

entity cycle_exactnote_reg is
  port (clk : in std_logic; d : in std_logic; q : out std_logic);
end;

architecture a of cycle_exactnote_reg is
begin
  process (clk)
  begin
    if rising_edge(clk) then
      q <= d;
    end if;
  end process;
end;

library ieee;
use ieee.std_logic_1164.all;

entity cycle_exactnote is
end;

architecture a of cycle_exactnote is
  signal clk, d : std_logic := '0';
  signal q : std_logic;
begin
  m : entity work.cycle_exactnote_mon port map (clk, d);
  g : entity work.cycle_exactnote_reg port map (clk, d, q);

  chk : process (clk)
  begin
    if rising_edge(clk) then
      report "top q=" & to_string(q);
    end if;
  end process;

  process
  begin
    for i in 1 to 3 loop
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
      d <= not d;
    end loop;
    wait;
  end process;
end;
