-- difftest: top=call_chain_error std=2008 exit=1 ignore=path
-- A division by zero three calls deep, after many calls that succeed: the error names the whole call
-- chain (a compiled tier must hand the interpreter every frame). ignore=path: #506.
entity call_chain_error is end;
architecture t of call_chain_error is
  function dec(x : natural) return natural is begin return 1000 / x; end function;
  function mid(x : natural) return natural is begin return dec(x) + 1; end function;
  function top(x : natural) return natural is begin return mid(x) * 2; end function;
begin
  process
    variable s : natural := 0;
  begin
    for i in 1 to 3000 loop s := s + top(i); end loop;
    report "s=" & integer'image(s);
    for i in 0 to 5 loop s := top(5 - i); end loop;
    report "unreachable";
    wait;
  end process;
end;
