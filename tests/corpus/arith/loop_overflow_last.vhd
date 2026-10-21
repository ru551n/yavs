-- difftest: top=loop_overflow_last std=2008 exit=1
-- A loop whose last iteration overflows INTEGER; compiled code must leave with the live
-- registers in place so the error names the same operands as the interpreter's.
entity loop_overflow_last is end;
architecture t of loop_overflow_last is
begin
  process
    variable x   : integer := 1;
    variable sum : integer := 0;
  begin
    report "start";
    for i in 1 to 20 loop
      sum := (sum + x) mod 1000;
      x := x * 3;
    end loop;
    report "unreachable";
    wait;
  end process;
end;
