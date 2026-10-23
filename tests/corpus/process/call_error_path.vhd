-- difftest: top=call_error_path std=2008 exit=1
-- A run-time error inside a function called from a process names that process (#506): the
-- fatal error of the call below, after the report of the call before it.
entity call_error_path is end entity;

architecture test of call_error_path is
  function inv(x : natural) return natural is
  begin
    return 1000 / x;
  end function;
begin
  process
    variable s : natural := 0;
  begin
    s := inv(4);
    report "s=" & integer'image(s);
    s := inv(0);
    report "unreachable";
    wait;
  end process;
end architecture;
