-- difftest: top=finish_in_function_call_order std=2008 exit=0
use std.env.all;
entity finish_in_function_call_order is end;
architecture t of finish_in_function_call_order is
begin
  process begin
    report "a"; wait for 1 ns; report "b";
    finish(0);
  end process;
end;
