-- difftest: top=aggregate_target_elems_order std=2008 ref=hand
-- Element choices of an aggregate target written out of index order, and over an index
-- subtype that is descending (#264, lrm-checks L68): the aggregate's direction is its index
-- subtype's (LRM 9.3.3.3), so the choices name their elements wherever they are written and
-- the leftmost index takes the value's leftmost element. ref=hand: the reference simulator
-- maps the associations in the order written; the other one that accepts the form follows
-- the index subtype, as below.
entity aggregate_target_elems_order is end;
architecture t of aggregate_target_elems_order is
  type iv is array (natural range <>) of integer;
  subtype di is integer range 3 downto 0;
  type dv is array (di range <>) of integer;
begin
  process
    variable vd : iv(1 downto 0) := (10, 0);
    variable ve : dv(3 downto 2) := (10, 0);
    variable a, b : integer;
  begin
    (1 => a, 0 => b) := vd;
    report "a=" & integer'image(a) & " b=" & integer'image(b);
    assert a = 0 and b = 10 severity failure;
    (2 => a, 3 => b) := ve;
    report "a=" & integer'image(a) & " b=" & integer'image(b);
    assert a = 0 and b = 10 severity failure;
    (3 => a, 2 => b) := ve;
    report "a=" & integer'image(a) & " b=" & integer'image(b);
    assert a = 10 and b = 0 severity failure;
    wait;
  end process;
end;
