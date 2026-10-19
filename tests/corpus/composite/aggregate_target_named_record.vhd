-- difftest: top=aggregate_target_named_record std=2008 ref=hand
-- Named record fields in aggregate targets (#350, LRM 10.5.2.1, 10.6.2.1; lrm-checks L98): the
-- type of the target comes from the right-hand side, a field name names the field whatever
-- the order written, for variables and signals, nested and with a function result.
-- ref=hand: the first reference simulator assigns by position and ignores the field names
-- (and drives a signal with the wrong field), so the .expected is the LRM result.
entity aggregate_target_named_record is end entity;
architecture t of aggregate_target_named_record is
  type inner is record
    a : integer;
    b : integer;
  end record;
  type outer is record
    i : inner;
    c : integer;
  end record;
  function mk(k : integer) return inner is
  begin
    return (a => k, b => k + 1);
  end function;
  function mko(k : integer) return outer is
  begin
    return (i => mk(k), c => k + 2);
  end function;
  signal sx, sy, sz : integer := 0;
begin
  process
    variable x, y, z : integer;
  begin
    (b => y, a => x) := inner'(1, 2);
    assert x = 1 and y = 2 severity failure;
    (b => y, a => x) := mk(3);
    assert x = 3 and y = 4 severity failure;
    (a => y, b => x) := mk(5);
    assert x = 6 and y = 5 severity failure;
    (c => z, i => (b => y, a => x)) := mko(7);
    assert x = 7 and y = 8 and z = 9 severity failure;
    ((b => y, a => x), z) := mko(10);
    assert x = 10 and y = 11 and z = 12 severity failure;
    (b => sy, a => sx) <= mk(1);
    wait for 1 ns;
    assert sx = 1 and sy = 2 severity failure;
    (a => sy, b => sx) <= inner'(3, 4) after 1 ns;
    wait for 2 ns;
    assert sx = 4 and sy = 3 severity failure;
    (c => sz, i => (b => sy, a => sx)) <= mko(20);
    wait for 1 ns;
    assert sx = 20 and sy = 21 and sz = 22 severity failure;
    report "x=" & integer'image(x) & " y=" & integer'image(y) & " sx=" & integer'image(sx) & " sy=" & integer'image(sy);
    wait;
  end process;
end architecture;
