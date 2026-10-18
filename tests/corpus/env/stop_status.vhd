-- difftest: top=stop_status std=2008 exit=2
use std.env.all;
entity stop_status is end;
architecture t of stop_status is
begin
  process begin
    wait for 4 ns;
    report "stopping";
    stop(2);
    wait;
  end process;
end;
