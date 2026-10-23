-- difftest: top=shared_plain std=1993
-- A shared variable of any type is legal in VHDL-93 (no protected types), without relaxed rules.
entity shared_plain is end;

architecture a of shared_plain is
  type counts is record
    ups, downs : integer;
  end record;
  shared variable total : integer := 0;
  shared variable c : counts := (0, 0);
begin
  up : process
  begin
    for i in 1 to 3 loop
      wait for 10 ns;
      total := total + 2;
      c.ups := c.ups + 1;
    end loop;
    wait;
  end process;

  down : process
  begin
    wait for 5 ns;
    for i in 1 to 3 loop
      wait for 10 ns;
      total := total - 1;
      c.downs := c.downs + 1;
    end loop;
    wait;
  end process;

  check : process
  begin
    wait for 100 ns;
    report "total " & integer'image(total) & " ups " & integer'image(c.ups)
           & " downs " & integer'image(c.downs);
    assert total = 3 and c.ups = 3 and c.downs = 3 severity failure;
    wait;
  end process;
end architecture;
