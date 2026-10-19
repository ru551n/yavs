-- difftest: top=wait_forms std=2008 ignore=order
-- ignore=order: processes report in one cycle, ordered by yavs's rule (#252), which the
-- reference simulator does not follow here, so each process's reports are compared.
entity wait_forms is end;
architecture t of wait_forms is
  signal a, b : integer := 0;
  signal go : boolean := false;
begin
  driver : process begin
    wait for 10 ns; a <= 1;
    wait for 10 ns; b <= 5;
    wait for 10 ns; a <= 2; b <= 6;
    wait for 10 ns; go <= true;
    wait;
  end process;
  on_a : process begin
    wait on a; report "a=" & integer'image(a) & " @" & time'image(now);
    wait on a, b; report "a or b: " & integer'image(a) & "," & integer'image(b) & " @" & time'image(now);
    wait on a, b; report "a or b again @" & time'image(now);
    wait;
  end process;
  until_p : process begin
    wait until a = 2 and b = 6;
    report "until a=2 b=6 @" & time'image(now);
    wait until go for 100 ns;
    report "until go @" & time'image(now);
    wait until go for 5 ns;
    report "timeout @" & time'image(now);
    wait;
  end process;
  timeout_p : process begin
    wait on a for 5 ns; report "timeout1 @" & time'image(now);
    wait on a for 50 ns; report "event @" & time'image(now);
    wait for 0 ns; report "zero wait delta";
    wait;
  end process;
end;
