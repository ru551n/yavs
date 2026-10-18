-- difftest: top=int_overflow std=2008 exit=1
entity int_overflow is end;
architecture t of int_overflow is
begin
  process
    variable a : integer := integer'high;
  begin
    report "start";
    a := a + 1;
    report "unreachable";
    wait;
  end process;
end;
