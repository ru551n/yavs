-- difftest: top=aggregate_2d_named_rows std=2008
-- A two-dimensional constant aggregate with nested named rows: the inner choices are of the
-- second index type, not the first (#457).
entity aggregate_2d_named_rows is end;
architecture t of aggregate_2d_named_rows is
  type remains is (r0, r1, r2);
  type branch is array (remains, bit) of remains;
  constant br_table : branch := (r0 => ('0' => r0, '1' => r1),
                                 r1 => ('0' => r2, '1' => r1),
                                 r2 => ('0' => r2, '1' => r0));
  type cube is array (bit, remains, boolean) of integer;
  constant cb : cube := ('0' => (r0 => (false => 1, true => 2), others => (others => 3)),
                         '1' => (others => (false => 4, true => 5)));
begin
  process
  begin
    assert br_table(r1, '0') = r2 severity failure;
    assert br_table(r2, '1') = r0 severity failure;
    assert cb('0', r0, true) = 2 and cb('0', r2, false) = 3 and cb('1', r1, true) = 5
      severity failure;
    report remains'image(br_table(r1, '0')) & " " & integer'image(cb('1', r2, false));
    wait;
  end process;
end architecture;
