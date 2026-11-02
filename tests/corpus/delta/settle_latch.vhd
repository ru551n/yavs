-- difftest: top=settle_latch std=2008
-- A pulse made by a delta skew is captured by a level latch (#1064): the latch holds state, so
-- running the pulse generator only once its inputs settled would lose the capture.
entity settle_latch is end;
architecture t of settle_latch is
  signal a, a_d, en, d, q : bit := '0';
begin
  a <= '1' after 1 ns, '0' after 3 ns;
  d <= '1' after 1 ns;
  dly : process (a) begin a_d <= a; end process;
  pulse : process (a, a_d) begin en <= a and not a_d; end process;
  latch : process (en, d) begin
    if en = '1' then q <= d; end if;
  end process;
  check : process begin
    wait for 2 ns;
    report "q=" & to_string(q);
    assert q = '1' report "the pulse was not captured" severity failure;
    wait;
  end process;
end;
