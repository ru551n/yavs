-- difftest: top=time_arith std=2008
entity time_arith is end;
architecture t of time_arith is
begin
  process
    variable t : time := 7 ns;
  begin
    report time'image(t + 3 ns) & " " & time'image(t - 10 ns) & " " & time'image(t * 3) & " " & time'image(t / 2);
    report time'image(t * 1.5) & " " & time'image(t / 2.0) & " ratio " & integer'image(t / 1 ns);
    report time'image(t rem 3 ns) & " " & time'image(t mod 3 ns);
    assert 1 ns / 1 ps = 1000 severity failure;
    assert real(t / 1 ps) = 7000.0 severity failure;
    assert maximum(1 ns, 2 ns) = 2 ns and minimum(1 ns, 2 ns) = 1 ns severity failure;
    wait;
  end process;
end;
