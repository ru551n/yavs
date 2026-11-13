entity top is end;
architecture a of top is
  signal s : real := 0.0;
  signal sv : real_vector(0 to 1) := (0.0, 1.0);
  signal n, nv : integer := 0;
begin
  process (s) begin n <= n + 1; end process;
  process (sv) begin nv <= nv + 1; end process;
  process
    variable z : real := 0.0;
  begin
    z := -z;  -- negative zero
    wait for 1 ns;
    s <= z;
    sv <= (z, 1.0);
    wait for 0 ns;
    report "RES event " & boolean'image(s'event) & " " & boolean'image(sv'event);
    report "RES active " & boolean'image(s'active) & " " & boolean'image(sv'active);
    wait for 1 ns;
    report "RES counts " & integer'image(n) & " " & integer'image(nv);
    wait;
  end process;
end;
