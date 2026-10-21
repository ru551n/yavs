-- difftest: top=unconstrained_records std=2008 ref=hand
-- ref=hand: sim1 gets this wrong (see README); .expected is hand-corrected to the LRM result.
entity unconstrained_records is end;
architecture t of unconstrained_records is
  type vec is array (natural range <>) of integer;
  type packet is record
    id : integer;
    data : vec;
  end record;
  type packets is array (natural range <>) of packet(data(0 to 1));
begin
  process
    variable p : packet(data(1 to 3)) := (id => 7, data => (10, 20, 30));
    variable ps : packets(1 to 2) := (others => (id => 1, data => (5, 6)));
  begin
    report "id=" & integer'image(p.id) & " n=" & integer'image(p.data'length) & " d(2)=" & integer'image(p.data(2));
    ps(2).id := 9; ps(1).data(1) := 55;
    report integer'image(ps(1).data(1)) & " " & integer'image(ps(2).id) & " " & integer'image(ps(2).data(1));
    wait;
  end process;
end;
