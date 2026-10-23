-- Three clock generators (4, 5 and 6 ns half periods) that edge together every now and then, a
-- counter per clock and a process whose timeouts fall on those edges: the cycles with several
-- entries due at one time stay on the kernel's timed update path (no general update).
library ieee; use ieee.std_logic_1164.all;
entity clk_together is end;
architecture a of clk_together is
  signal ca, cb, cc : std_logic := '0';
  signal na, nb, nc, nt : natural := 0;
  signal done : boolean := false;
begin
  ca <= not ca after 4 ns when not done;
  cb <= not cb after 5 ns when not done;
  cc <= not cc after 6 ns when not done;
  process (ca) begin if rising_edge(ca) then na <= na + 1; end if; end process;
  process (cb) begin if rising_edge(cb) then nb <= nb + 1; end if; end process;
  process (cc) begin if rising_edge(cc) then nc <= nc + 1; end if; end process;
  process begin
    for i in 1 to 40 loop wait for 10 ns; nt <= nt + 1; end loop;
    wait for 1 ns;
    report "n=" & natural'image(na) & "," & natural'image(nb) & "," & natural'image(nc) & "," & natural'image(nt);
    done <= true; wait;
  end process;
end;
