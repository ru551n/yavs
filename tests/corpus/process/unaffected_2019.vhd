-- difftest: top=unaffected_2019 std=2019 ref=hand
-- 2019 `unaffected` in simple variable and force assignments (10.6.2.1, 10.5.2.1):
-- the target keeps its value (a force: nothing is forced). ref=hand: the reference
-- simulator 1.23 assigns the next alternative instead and forces an undefined value;
-- the expected output is the LRM result.
entity unaffected_2019 is end;
architecture t of unaffected_2019 is
  signal f : integer := 0;
begin
  process
    variable v : integer := 21;
    variable w : bit_vector(3 downto 0) := "0101";
  begin
    v := unaffected when v = 21 else 0;
    report "kept " & integer'image(v);
    v := 5 when v = 0 else unaffected;
    report "kept " & integer'image(v);
    v := 7 when v = 0 else 8 when v = 21 else unaffected when v = 22 else 9;
    report "chain " & integer'image(v);
    v := 7 when v = 0 else unaffected when v = 8 else 9;
    report "chain " & integer'image(v);
    w := (others => '0') when w(0) = '1' else unaffected;
    assert w = "0000" severity failure;
    w := (others => '1') when w(0) = '1' else unaffected;
    assert w = "0000" severity failure;
    f <= force 42 when v = 8 else unaffected;
    wait for 1 ns;
    report "forced " & integer'image(f);
    f <= release;
    f <= force unaffected when v = 8 else 1;
    wait for 1 ns;
    report "not forced " & integer'image(f);
    wait;
  end process;
end;
