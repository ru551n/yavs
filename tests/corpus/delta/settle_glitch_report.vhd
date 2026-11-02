-- difftest: top=settle_glitch_report std=2008
-- An assertion that fires on a value that lives for one delta (#1064): both signals are '1'
-- only in the delta between their changes; the report shows that delta.
entity settle_glitch_report is end;
architecture t of settle_glitch_report is
  signal a, a_d, x, y : bit := '0';
begin
  a <= '1' after 1 ns, '0' after 2 ns, '1' after 3 ns, '0' after 4 ns, '1' after 5 ns;
  dly : process (a) begin a_d <= a; end process;
  px : process (a) begin x <= a; end process;
  py : process (a_d) begin y <= not a_d; end process;
  mon : process (x, y) begin
    assert not (x = '1' and y = '1') report "x and y both high" severity note;
  end process;
end;
