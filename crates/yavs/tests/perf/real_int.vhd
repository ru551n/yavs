-- Reals rounded to integers in a function on a native frame and in a process: compiled code
-- rounds them itself, no step of the host.
entity real_int is end;
architecture a of real_int is
  subtype small is integer range -1000 to 1000;
  function scaled(x : real) return small is
  begin
    return small(x * 7.5);
  end function;
begin
  process
    variable s : integer := 0;
    variable x : real := -60.25;
  begin
    for k in 1 to 200 loop
      x := x + 0.625;
      s := (s * 3 + scaled(x) + integer(x / 3.0)) mod 99991;
      wait for 1 ns;
    end loop;
    report "s=" & integer'image(s);
    wait;
  end process;
end;
