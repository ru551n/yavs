-- difftest: top=pulse_swallow std=2008
entity pulse_swallow is end;
architecture t of pulse_swallow is
  signal i, o_in, o_tr : bit := '0';
begin
  o_in <= i after 5 ns;
  o_tr <= transport i after 5 ns;
  process begin
    wait for 10 ns; i <= '1';
    wait for 2 ns; i <= '0';
    wait for 20 ns; i <= '1';
    wait for 8 ns; i <= '0';
    wait;
  end process;
  process (o_in) begin report "inertial " & bit'image(o_in) & " @" & time'image(now); end process;
  process (o_tr) begin report "transport " & bit'image(o_tr) & " @" & time'image(now); end process;
end;
