-- difftest: top=real_attrs std=2008
entity real_attrs is end;
architecture t of real_attrs is
  subtype r is real range 0.0 to 10.0;
begin
  process
    variable x : real := 2.5;
  begin
    assert r'low = 0.0 and r'high = 10.0 and r'ascending severity failure;
    assert real'high > 1.0e300 and real'low < -1.0e300 severity failure;
    assert integer(x) = 3 and integer(-x) = -3 and integer(3.4) = 3 severity failure;
    assert real(3) = 3.0 and x * 2.0 = 5.0 severity failure;
    assert abs(-x) = 2.5 and x ** 2 = 6.25 severity failure;
    report "value=" & integer'image(integer(real'value("2.25") * 100.0));
    wait;
  end process;
end;
