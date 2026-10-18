-- difftest: top=finish_noarg std=2019
use std.env.all;
entity finish_noarg is end;
architecture t of finish_noarg is
  signal clk : bit := '0';
begin
  clk <= not clk after 5 ns;
  process begin
    wait for 22 ns;
    report "finishing at " & time'image(now);
    finish;
  end process;
end;
