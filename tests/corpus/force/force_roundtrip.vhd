-- difftest: top=force_roundtrip std=2008
-- Two forces in one cycle, the last back to the current value, still wake the readers (#382).
entity force_roundtrip is end;
architecture a of force_roundtrip is
  signal s, cnt : natural;
begin
  process (s) begin cnt <= cnt + 1; end process;
  process
  begin
    wait for 1 ns;
    s <= force 1;
    wait for 0 ns;
    wait for 0 ns;
    s <= force 2;
    s <= force 1;
    wait for 0 ns;
    wait for 0 ns;
    assert cnt = 3 report "cnt = " & integer'image(cnt) severity failure;
    report "done";
    wait;
  end process;
end;
