entity top is end;
architecture a of top is
  signal s, cnt : natural;
begin
  process (s) begin cnt <= cnt + 1; end process;
  process
  begin
    wait for 1 ns;
    s <= force 1;
    wait for 0 ns;
    wait for 0 ns;
    -- Two forces in one cycle, the last back to the current value.
    s <= force 2;
    s <= force 1;
    wait for 0 ns;
    wait for 0 ns;
    report "RES round trip cnt = " & integer'image(cnt);
    -- The same value forced again, once and twice: no event.
    s <= force 1;
    s <= force 1;
    wait for 0 ns;
    wait for 0 ns;
    report "RES same value cnt = " & integer'image(cnt);
    wait;
  end process;
end;
