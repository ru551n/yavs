-- difftest: top=finish_status std=2008 exit=3
use std.env.all;
entity finish_status is end;
architecture t of finish_status is
begin
  process begin
    report "working";
    wait for 4 ns;
    finish(3);
    report "unreachable";
    wait;
  end process;
  process begin wait for 10 ns; report "must not run"; wait; end process;
end;
