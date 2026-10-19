-- difftest: top=aggregate_target_range_order std=2008 ref=hand
-- Named associations in an aggregate target in any order (#244, lrm-checks L68): a choice
-- names its elements wherever it is written (LRM 9.3.3.3), so the result does not depend on
-- the order of the associations. ref=hand: the reference simulator maps the associations in
-- the order they are written, as if they were positional; the other simulator that accepts
-- range choices in targets gives the results below.
entity aggregate_target_range_order is end;
architecture t of aggregate_target_range_order is
  type iv is array (natural range <>) of integer;
begin
  process
    variable vd : iv(5 downto 0) := (50, 40, 30, 20, 10, 0);
    variable hi : iv(5 downto 2);
    variable lo : iv(1 downto 0);
    variable e0 : integer;
    variable m : iv(5 downto 1);
  begin
    (1 downto 0 => lo, 5 downto 2 => hi) := vd;
    report "hi(5)=" & integer'image(hi(5)) & " lo(1)=" & integer'image(lo(1));
    assert hi = (50, 40, 30, 20) and lo = (10, 0) severity failure;
    (0 => e0, 5 downto 1 => m) := vd;
    report "e0=" & integer'image(e0) & " m(5)=" & integer'image(m(5));
    assert e0 = 0 and m = (50, 40, 30, 20, 10) severity failure;
    wait;
  end process;
end;
