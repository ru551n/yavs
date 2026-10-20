-- difftest: top=tb_last_value_whole std=2008
-- 'last_value of a composite signal is the value of the whole signal before its last event
-- (an element that did not change in that event keeps its current value).
entity tb_last_value_whole is end;
architecture t of tb_last_value_whole is
  function get_last_value (signal x : bit_vector) return bit_vector is
  begin
    return x'last_value;
  end function;
  signal s : bit_vector(1 to 3);
begin
  s <= "101" after 1 ns, "011" after 2 ns;
  p1 : process begin
    wait for 1 ns;
    assert get_last_value(s) = "000";
    wait for 1 ns;
    assert get_last_value(s) = "101";
    assert s'last_value = "101";
    report "ok";
    wait;
  end process;
end;
