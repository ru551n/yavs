-- 'active of a signal that another process assigns in the delta cycle where the reader runs: the
-- transaction matures in the next delta cycle, so the signal is not active yet (LRM 14.7.3.2, 16.2.4).
entity top is end entity;
architecture t of top is
  signal a : bit := '0';
  signal b : bit := '0';
begin
  stim : process is
  begin
    wait for 10 ns;
    a <= '1';
    wait;
  end process;

  echo : process (a) is
  begin
    b <= a;
  end process;

  watch : process is
  begin
    wait on a;
    report "A b'active=" & boolean'image(b'active) & " b'event=" & boolean'image(b'event);
    wait for 0 ns;
    report "B b'active=" & boolean'image(b'active) & " b'event=" & boolean'image(b'event);
    wait;
  end process;
end architecture;
