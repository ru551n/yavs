-- difftest: top=aggregate_target_elems_cover std=2008 exit=1
-- An aggregate target of element choices must cover its value exactly (LRM 10.6.2.1): a value
-- one element longer than the choices is a runtime error, raised before any element is
-- assigned.
entity aggregate_target_elems_cover is end;
architecture t of aggregate_target_elems_cover is
  type iv is array (natural range <>) of integer;
begin
  process
    variable vo : iv(5 downto 3) := (30, 20, 10);
    variable a, b : integer := 0;
    variable n : natural := 4;
  begin
    (0 => a, 1 => b) := vo(n downto 3);
    report "a=" & integer'image(a) & " b=" & integer'image(b);
    n := n + 1;
    (0 => a, 1 => b) := vo(n downto 3);
    report "not reached";
    wait;
  end process;
end;
