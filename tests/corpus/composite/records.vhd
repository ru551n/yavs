-- difftest: top=records std=2008
entity records is end;
architecture t of records is
  type point is record x, y : integer; end record;
  type shape is record
    name : string(1 to 4);
    p : point;
    tags : bit_vector(0 to 2);
  end record;
  constant origin : point := (0, 0);
  function add(a, b : point) return point is begin return (a.x + b.x, a.y + b.y); end;
begin
  process
    variable s : shape := (name => "tri ", p => (3, 4), tags => "101");
    variable q : point;
  begin
    q := add(s.p, (1, 1));
    report s.name & integer'image(q.x) & "," & integer'image(q.y);
    s.p.x := 10; s.tags(1) := '1';
    report integer'image(s.p.x) & " " & to_string(s.tags);
    assert s.p /= origin and s.p = (10, 4) severity failure;
    s := (name => "sq  ", p => origin, tags => (others => '0'));
    report s.name & to_string(s.tags) & boolean'image(s.p = origin);
    wait;
  end process;
end;
