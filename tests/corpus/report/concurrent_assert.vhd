-- difftest: top=concurrent_assert std=2019
entity concurrent_assert is end;
architecture t of concurrent_assert is
  signal s : integer := 0;
begin
  assert s < 3 report "s too large: " & integer'image(s) severity warning;
  process begin
    for i in 1 to 4 loop wait for 1 ns; s <= i; end loop;
    wait;
  end process;
end;
