-- difftest: top=aggregate_target_cover std=2008 exit=1
-- An aggregate target with 2008 slice pieces must cover its value exactly (#230, LRM
-- 10.6.2.1): a value one element longer than the pieces is a runtime error, raised before
-- any piece is assigned.
entity aggregate_target_cover is end;
architecture t of aggregate_target_cover is
  type iv is array (natural range <>) of integer;
begin
  process
    variable x : iv(0 to 3) := (1, 2, 3, 4);
    variable y : iv(0 to 1) := (0, 0);
    variable z : integer := 0;
    variable k : natural := 2;
  begin
    (y, z) := x(0 to k);
    report "y(0)=" & integer'image(y(0)) & " z=" & integer'image(z);
    k := k + 1;
    (y, z) := x(0 to k);
    report "not reached";
    wait;
  end process;
end;
