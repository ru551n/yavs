-- A procedure that can wait (its wait behind a condition that holds once): its integer locals
-- live in registers wherever no wait leaves them live, as in a procedure that cannot wait, so a
-- call that does not wait runs its loop without a load and store of the frame per statement.
entity wait_proc_slots is end;
architecture a of wait_proc_slots is
  procedure step(variable r : inout integer; go : boolean) is
    variable l1 : integer := 1;
    variable l2 : integer := 2;
  begin
    for i in 1 to 4 loop
      l1 := (l1 * 7 + i) mod 1009;
      l2 := (l2 * 5 + l1) mod 1013;
    end loop;
    if go then
      wait for 1 ns;
    end if;
    r := (r + l1 + l2) mod 100003;
  end procedure;
begin
  process
    variable r : integer := 1;
  begin
    for k in 1 to 200 loop
      step(r, k = 100);
    end loop;
    report "r=" & integer'image(r) & " now=" & time'image(now);
    wait;
  end process;
end;
