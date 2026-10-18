-- difftest: top=min_literal std=2008 ref=hand
-- LRM 9.3, 5.2.3: the minus applies to a universal_integer literal, so the i64 minimum is legal
-- ref=hand: the reference simulators reject the minimum literal (their universal_integer is 64-bit signed); the LRM leaves the range implementation-defined, yavs accepts it
entity min_literal is end;
architecture t of min_literal is
  type big is range -9223372036854775808 to 9223372036854775807;
  constant a : big := -9223372036854775808;
  constant c : time := -9223372036854775808 fs;
begin
  process begin
    assert a = big'low and c = time'low severity failure;
    assert a + 1 = -9223372036854775807 severity failure;
    report "a=" & big'image(a);
    report "c=" & time'image(c);
    wait;
  end process;
end;
