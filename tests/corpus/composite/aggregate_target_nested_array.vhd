-- difftest: top=aggregate_target_nested_array std=2008 ref=hand
-- Nested aggregate targets of arrays of arrays (#350, LRM 10.5.2.1, 10.6.2.1; lrm-checks L98):
-- every element of the value goes to its own target element, rows taken by one name or split
-- into scalars, positional or by index choice, for variables and signals.
-- ref=hand: the first reference simulator gives every row the first row's elements (and
-- ignores index choices), so the .expected is the LRM result.
entity aggregate_target_nested_array is end entity;
architecture t of aggregate_target_nested_array is
  type row is array (0 to 1) of integer;
  type mat is array (0 to 2) of row;
  type mrec is record
    r : row;
    k : integer;
  end record;
  type rows is array (0 to 1) of mrec;
  function mk(b : integer) return mat is
  begin
    return ((b, b + 1), (b + 2, b + 3), (b + 4, b + 5));
  end function;
  signal s0, s1, s2, s3, s4, s5 : integer := 0;
  signal sr : row := (0, 0);
begin
  process
    variable p0, p1, p2, p3, p4, p5 : integer;
    variable w : row;
    variable m : mat;
    variable rr : rows;
  begin
    ((p0, p1), (p2, p3), (p4, p5)) := mk(10);
    assert p0 = 10 and p1 = 11 and p2 = 12 and p3 = 13 and p4 = 14 and p5 = 15 severity failure;
    (w, (p2, p3), (p4, p5)) := mk(20);
    assert w = (20, 21) and p2 = 22 and p3 = 23 and p4 = 24 and p5 = 25 severity failure;
    ((p0, p1), w, (p4, p5)) := mat'((30, 31), (32, 33), (34, 35));
    assert p0 = 30 and p1 = 31 and w = (32, 33) and p4 = 34 and p5 = 35 severity failure;
    (2 => (p5, p4), 1 => w, 0 => (p1, p0)) := mk(40);
    assert p1 = 40 and p0 = 41 and w = (42, 43) and p5 = 44 and p4 = 45 severity failure;
    ((w, p0), (m(1), p1)) := rows'(((50, 51), 52), ((53, 54), 55));
    assert w = (50, 51) and p0 = 52 and m(1) = (53, 54) and p1 = 55 severity failure;
    ((s0, s1), (s2, s3), (s4, s5)) <= mk(60);
    wait for 1 ns;
    assert s0 = 60 and s1 = 61 and s2 = 62 and s3 = 63 and s4 = 64 and s5 = 65 severity failure;
    (sr, (s2, s3), (s4, s5)) <= mk(70) after 1 ns;
    wait for 2 ns;
    assert sr = (70, 71) and s2 = 72 and s3 = 73 and s4 = 74 and s5 = 75 severity failure;
    (2 => (s5, s4), 1 => sr, 0 => (s1, s0)) <= mk(80);
    wait for 1 ns;
    assert s1 = 80 and s0 = 81 and sr = (82, 83) and s5 = 84 and s4 = 85 severity failure;
    report "p0=" & integer'image(p0) & " s0=" & integer'image(s0) & " s5=" & integer'image(s5);
    wait;
  end process;
end architecture;
