-- difftest: top=now_in_bounds ref=hand
-- ref=hand: only a report, no reference-specific output.
-- `now` is 0 fs during elaboration, so a constant that reads it can bound a port or signal
-- subtype (#620): L is 9 and the alias has 7 elements.
entity now_in_bounds is end entity;
architecture h of now_in_bounds is
  constant L : positive := 9 + now / 1 ns;
  signal   s : bit_vector(31 downto 0);
  alias    a : bit_vector(L-1 downto 2) is s(L-1 downto 2);
  signal   b : bit_vector(L-1 downto 2);
begin
  a <= b;
  process begin
    report "L = " & integer'image(L) & " a'length = " & integer'image(a'length);
    assert L = 9 severity failure;
    assert a'length = 7 severity failure;
    wait;
  end process;
end architecture;
