-- difftest: top=real_to_int_range std=2008 exit=1
entity real_to_int_range is end;
architecture t of real_to_int_range is
begin
  process
    variable r : real := 3.0e10;
    variable i : integer;
  begin
    report "start";
    i := integer(r);
    report "unreachable";
    wait;
  end process;
end;
