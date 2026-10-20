-- A run killed by SIGINT or SIGTERM keeps the reports it made (#428). The design reports,
-- tells the test through a marker file that the report is out, then computes for ever.
library std;
use std.textio.all;

entity signal_flush is
  generic (ready : string);
end entity;

architecture a of signal_flush is
begin
  process
    file f : text;
    variable l : line;
    variable x : integer := 0;
  begin
    report "before signal";
    wait for 1 ns;
    file_open(f, ready, write_mode);
    write(l, string'("ready"));
    writeline(f, l);
    file_close(f);
    for j in 1 to 1000000 loop
      for i in 1 to 100000000 loop
        x := (x + 1) mod 1000;
      end loop;
    end loop;
    wait;
  end process;
end architecture;
