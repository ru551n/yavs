-- A function calls a procedure of another unit that waits (LRM 10.2, #823).
package wp is
  procedure call_wait (constant dly : in time);
end;
package body wp is
  procedure call_wait (constant dly : in time) is begin wait for dly; end;
end;
use work.wp.all;
entity function_calls_waiting_procedure is end;
architecture a of function_calls_waiting_procedure is
  function call_waiter (constant dly : in time) return time is
  begin
    call_wait(dly);
    return dly * 2;
  end;
begin
  process
    variable delay : time := 2 ns;
  begin
    delay := call_waiter(delay);
    wait;
  end process;
end;
