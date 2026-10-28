-- yavs-test: std=2008
-- expect: E1801 11:5 procedure `indirect_wait` contains a `wait` and is called by function `call_waiter`
-- LRM 10.2: a function calls no procedure that waits, directly or through other procedures (#823)
entity e is end;
architecture a of e is
  procedure call_wait (constant dly : in time) is begin wait for dly; end;
  procedure indirect_wait (constant dly : in time) is begin call_wait(dly); end;
  procedure no_wait (constant dly : in time) is begin null; end;
  function call_waiter (constant dly : in time) return time is
  begin
    indirect_wait(dly);
    no_wait(dly);
    return dly * 2;
  end;
begin
end;
