-- difftest: top=idle_when std=2008
-- Concurrent conditional assignments whose arms cannot be taken do not wake for the signals
-- they read (idle skip): a condition of a generic, a condition on a signal, and the two
-- together. 300 cycles; `a` changes every cycle, `sel` every 30.
library ieee; use ieee.std_logic_1164.all;
entity idle_when is
  generic (OFF : boolean := false; EN : boolean := true);
end;
architecture a of idle_when is
  signal clk : std_logic := '0';
  signal cnt : natural range 0 to 1023 := 0;
  signal a, z : natural range 0 to 1023 := 0;
  signal sel : std_logic := '0';
  signal t1, t2, t3, t4, t5, t6 : natural range 0 to 1023 := 0;
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  t1 <= a when OFF else 7;
  t2 <= a when EN else z;
  t3 <= a when sel = '1' else 5;
  t4 <= a when OFF and sel = '1' else z when sel = '0' else 3;
  t5 <= a when sel = '1' and EN else cnt when sel /= '1' and not OFF else 9;
  t6 <= a + z when (a > 3) and OFF else 11;
  process (clk)
  begin
    if rising_edge(clk) then
      cnt <= (cnt + 1) mod 1024;
      a <= (cnt * 3) mod 1024;
      z <= (cnt * 5) mod 1024;
      if cnt mod 30 = 29 then sel <= not sel; end if;
      if cnt = 300 then done <= true; end if;
    end if;
  end process;
  process (done)
  begin
    if done then
      report "sum=" & integer'image(t1 + t2 + t3 + t4 + t5 + t6);
    end if;
  end process;
end;
