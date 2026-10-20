-- difftest: top=record_signal_attrs std=2008 ignore=order ref=hand
-- ref=hand: 'last_value of s(1 to 2) is the value before the last event of that part (LRM 16.2.4), so
-- an element that did not change in it keeps its current value; the first reference simulator answers
-- per element (#451, catalogue case last-value-composite), the other two references agree with the
-- hand-edited line.
-- ignore=order: processes report in one cycle, ordered by yavs's rule (#252), which the
-- reference simulator does not follow here, so each process's reports are compared.
-- Signal attributes of records and of parts of arrays of records that span several fields:
-- 'event, 'active, 'last_event, 'last_active, 'last_value, and the implicit signals
-- 'stable, 'quiet, 'transaction and 'delayed of such parts (#215).
entity record_signal_attrs is
end entity;

architecture test of record_signal_attrs is
  type cell is record
    ok : boolean;
    n  : natural;
  end record;
  type cell_vec is array (natural range <>) of cell;
  signal s : cell_vec(0 to 2) := ((false, 0), (true, 1), (false, 2));
  signal r : cell := (true, 9);
  signal st, qu, rst : boolean;
  signal tr : bit;
  signal dl : cell;
  signal dv : cell_vec(0 to 1);

  function image(c : cell) return string is
  begin
    return "(" & boolean'image(c.ok) & "," & integer'image(c.n) & ")";
  end function;
begin
  st  <= s(1)'stable(2 ns);
  qu  <= s(1)'quiet(2 ns);
  tr  <= s(1)'transaction;
  dl  <= s(1)'delayed(3 ns);
  dv  <= s(1 to 2)'delayed(1 ns);
  rst <= r'stable;

  drv : process
    variable v2 : cell_vec(1 to 2);
  begin
    wait for 1 ns;
    s(1) <= (false, 5);
    wait for 0 ns;
    report "s(1)'event " & boolean'image(s(1)'event) & " s(0)'event " & boolean'image(s(0)'event)
      & " s(1 to 2)'event " & boolean'image(s(1 to 2)'event)
      & " s'active " & boolean'image(s'active)
      & " s(1)'last_value " & image(s(1)'last_value);
    wait for 4 ns;
    s(1).ok <= true;
    s(2).n <= 3;
    wait for 2 ns;
    v2 := s(1 to 2)'last_value;
    report "s(1)'last_event " & time'image(s(1)'last_event)
      & " s(0)'last_event " & time'image(s(0)'last_event)
      & " s(1)'last_active " & time'image(s(1)'last_active)
      & " s(1 to 2)'last_value(1) " & image(v2(1));
    s(1) <= (false, 6);
    r.n <= 3;
    wait for 2 ns;
    s(0).n <= 7;
    s(2) <= s(2);
    wait for 0 ns;
    report "s(2)'active " & boolean'image(s(2)'active) & " s(2)'event " & boolean'image(s(2)'event)
      & " r'last_event " & time'image(r'last_event) & " r'last_value " & image(r'last_value);
    wait;
  end process;

  -- One monitor, so that reports of the same cycle come in one order. The reference
  -- simulators differ on 'transaction at initialisation: count its toggles after it.
  mon : process (st, qu, rst, tr) is
    variable n : natural := 0;
  begin
    if st'event or qu'event or rst'event or now = 0 ns then
      report "st " & boolean'image(st) & " qu " & boolean'image(qu) & " rst " & boolean'image(rst);
    end if;
    if tr'event and now > 0 ns then
      n := n + 1;
      report "transaction " & integer'image(n);
    end if;
  end process;

  -- They also differ on 'delayed before its delay has passed once.
  delayed : process (dl, dv) is
  begin
    if now > 3 ns then
      report "dl " & image(dl) & " dv " & image(dv(0)) & image(dv(1));
    end if;
  end process;
end architecture;
