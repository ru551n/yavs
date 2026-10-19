-- difftest: top=aggregate_target_dynamic std=2008
-- An aggregate target whose only choice is a non-static element choice (LRM 9.3.3.3) takes
-- the value's single element whatever the value's bounds and the choice's value (#264, L68).
-- Variable and signal targets, choices from a variable and a generic-free signal.
entity aggregate_target_dynamic is end;
architecture t of aggregate_target_dynamic is
  type iv is array (natural range <>) of integer;
  signal s : integer := -1;
  signal j : natural := 4;
begin
  process
    variable vo : iv(5 downto 3) := (30, 20, 10);
    variable va : iv(0 to 3) := (5, 6, 7, 8);
    variable a : integer;
    variable i : natural := 1;
  begin
    (i => a) := vo(4 downto 4);
    report "a=" & integer'image(a);
    assert a = 20 severity failure;
    i := i + 6;
    (i => a) := va(i - 5 to i - 5);
    report "a=" & integer'image(a);
    assert a = 7 severity failure;
    (j => s) <= vo(3 downto 3);
    wait for 1 ns;
    report "s=" & integer'image(s);
    assert s = 10 severity failure;
    (i - 7 => s) <= iv'(9 => 7) after 1 ns;
    wait for 2 ns;
    report "s=" & integer'image(s);
    assert s = 7 severity failure;
    wait;
  end process;
end;
