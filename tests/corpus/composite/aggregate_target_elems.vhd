-- difftest: top=aggregate_target_elems std=2008
-- Element choices in an aggregate target map by position (#264, lrm-checks L68): the
-- aggregate's leftmost index takes the value's leftmost element whatever the value's bounds,
-- as for range choices. Variable and signal targets, values of both directions and with
-- bounds other than the choices'.
entity aggregate_target_elems is end;
architecture t of aggregate_target_elems is
  type iv is array (natural range <>) of integer;
  signal s0, s1, s2 : integer := -1;
begin
  process
    variable vd : iv(1 downto 0) := (10, 0);
    variable va : iv(0 to 1) := (10, 0);
    variable vo : iv(5 downto 3) := (30, 20, 10);
    variable a, b, c : integer;
  begin
    (0 => a, 1 => b) := vd;
    assert a = 10 and b = 0 severity failure;
    (0 => a, 1 => b) := va;
    assert a = 10 and b = 0 severity failure;
    (0 => a, 1 => b, 2 => c) := vo;
    assert a = 30 and b = 20 and c = 10 severity failure;
    (7 => a, 8 => b, 9 => c) := vo;
    assert a = 30 and b = 20 and c = 10 severity failure;
    (0 => s0, 1 => s1, 2 => s2) <= vo;
    wait for 1 ns;
    assert s0 = 30 and s1 = 20 and s2 = 10 severity failure;
    (0 => s0, 1 => s1) <= vd;
    wait for 1 ns;
    report "s0=" & integer'image(s0) & " s1=" & integer'image(s1);
    assert s0 = 10 and s1 = 0 severity failure;
    wait;
  end process;
end;
