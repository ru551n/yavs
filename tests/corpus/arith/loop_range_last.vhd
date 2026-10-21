-- difftest: top=loop_range_last std=2008 exit=1
-- A loop whose last iteration leaves the range of a subtype.
entity loop_range_last is end;
architecture t of loop_range_last is
  subtype small is integer range 0 to 100;
begin
  process
    variable v   : small := 0;
    variable cnt : integer := 0;
  begin
    report "start";
    for i in 1 to 21 loop
      cnt := cnt + i;
      v := v + 5;
    end loop;
    report "unreachable";
    wait;
  end process;
end;
