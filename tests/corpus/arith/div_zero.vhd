-- difftest: top=div_zero std=2008 exit=1
entity div_zero is end;
architecture t of div_zero is
begin
  process
    variable a : integer := 5; variable z : integer := 0;
  begin
    report "start";
    a := a / z;
    report "unreachable";
    wait;
  end process;
end;
