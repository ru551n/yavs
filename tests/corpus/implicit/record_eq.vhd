-- difftest: top=record_eq std=2008
-- Predefined "=" and "/=" on records, arrays of records and records with array fields (#163).
entity record_eq is end;
architecture a of record_eq is
  type r is record a : integer; b : bit_vector(1 to 2); end record;
  type ra is array (natural range <>) of r;
  type rp is record n : integer; s : string(1 to 3); end record;
  type rpa is array (natural range <>) of rp;
begin
  process
    variable x, y : r := (1, "10");
    variable u, v : ra(1 to 2) := (others => (1, "10"));
    variable p, q : rpa(1 to 2) := (others => (2, "abc"));
  begin
    assert x = y; assert u = v; assert p = q;
    y.a := 2; v(2).b := "11";
    assert x /= y; assert u /= v;
    q(1).s := "abd"; assert p /= q;
    report "equality ok";
    wait;
  end process;
end;
