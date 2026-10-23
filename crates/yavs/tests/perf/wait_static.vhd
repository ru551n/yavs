-- Processes that wait on the same signals every time (static waits) and the 'image of scalars:
-- the wait re-arms without looking its signals up again.
entity wait_static is end;
architecture a of wait_static is
  signal a, b : integer := 0;
  signal done : boolean := false;
begin
  process begin
    for k in 1 to 100 loop
      a <= k;
      wait for 1 ns;
      b <= k * 2;
      wait for 1 ns;
    end loop;
    done <= true; wait;
  end process;
  process
    variable s : natural := 0;
    variable l : string(1 to 6);
  begin
    wait on a, b, done;
    if done then
      report "s=" & integer'image(s);
      wait;
    end if;
    l := integer'image(a + b + 100000)(1 to 6);
    if l(1) = '1' then s := s + 1; end if;
  end process;
end;
