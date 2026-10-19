-- difftest: top=min_literal_2019 std=2019
-- LRM 5.2.3: the i64 minimum written without a literal beyond i64'high (such a literal is E1402)
entity min_literal_2019 is end;
architecture t of min_literal_2019 is
  type big is range -9223372036854775807 - 1 to 9223372036854775807;
  constant a : big := -9223372036854775807 - 1;
  constant c : time := -9223372036854775807 fs - 1 fs;
begin
  process begin
    assert a = big'low and c = time'low severity failure;
    assert a + 1 = -9223372036854775807 severity failure;
    report "a=" & big'image(a);
    report "c=" & time'image(c);
    wait;
  end process;
end;
