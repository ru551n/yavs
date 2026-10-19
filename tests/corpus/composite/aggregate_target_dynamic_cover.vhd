-- difftest: top=aggregate_target_dynamic_cover std=2008 exit=1
-- An aggregate target with a lone non-static element choice covers one element: a value of
-- two elements is a runtime error, before anything is assigned (LRM 10.6.2.1, #264).
entity aggregate_target_dynamic_cover is end;
architecture t of aggregate_target_dynamic_cover is
  type iv is array (natural range <>) of integer;
begin
  process
    variable vo : iv(5 downto 3) := (30, 20, 10);
    variable a : integer := 0;
    variable i : natural := 1;
  begin
    (i => a) := vo(i + 2 downto 3);
    report "a=" & integer'image(a);
    i := i + 1;
    (i => a) := vo(i + 2 downto 3);
    report "not reached";
    wait;
  end process;
end;
