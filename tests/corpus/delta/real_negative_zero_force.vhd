-- difftest: top=real_negative_zero_force std=2008 ref=hand
-- Forcing a REAL signal to -0.0 and releasing it back to 0.0 is no event: the values are equal
-- (LRM 9.2.3). The first reference simulator makes an event; the second one and this test do
-- not (#1145).
entity real_negative_zero_force is end;
architecture a of real_negative_zero_force is
  signal s, d : real := 0.0;
  signal n, m : integer := 0;
begin
  process (s) begin n <= n + 1; end process;
  process (d) begin m <= m + 1; end process;
  d <= s;
  drv : process
    variable z : real := 0.0;
  begin
    z := -z;
    wait for 1 ns;
    s <= force z;
    wait for 1 ns;
    report "forced n=" & integer'image(n) & " m=" & integer'image(m);
    s <= release;
    wait for 1 ns;
    report "released n=" & integer'image(n) & " m=" & integer'image(m);
    s <= force 1.0;
    wait for 1 ns;
    report "forced1 n=" & integer'image(n) & " m=" & integer'image(m);
    s <= force z;
    wait for 1 ns;
    report "forced-0 n=" & integer'image(n) & " m=" & integer'image(m);
    s <= release;
    wait for 1 ns;
    report "released n=" & integer'image(n) & " m=" & integer'image(m);
    wait;
  end process;
end;
