-- difftest: top=aggregate_target_matrix_cover std=2008 exit=1 ref=hand
-- A matrix aggregate target must cover its value in every dimension (LRM 10.5.2.1,
-- 10.6.2.1; #350): a value with three columns for a target of two is a run-time error before
-- anything is assigned.
-- ref=hand: the reference simulators report the mismatch with their own texts.
entity aggregate_target_matrix_cover is end entity;
architecture t of aggregate_target_matrix_cover is
  type m2 is array (natural range <>, natural range <>) of integer;
  signal s : m2(0 to 1, 0 to 2) := ((1, 2, 3), (4, 5, 6));
begin
  process
    variable p0, p1, p2, p3 : integer := 7;
  begin
    ((p0, p1), (p2, p3)) := s;
    report "not reached";
    wait;
  end process;
end architecture;
