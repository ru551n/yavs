-- difftest: top=record_signal_params std=2008 ignore=order
-- ignore=order: processes report in one cycle, ordered by yavs's rule (#252), which the
-- reference simulator does not follow here, so each process's reports are compared.
-- Signal parameters whose type has records in it: records, arrays of records and their parts
-- as actuals of in, out and inout signal parameters, passed on to further calls, with
-- attributes, waits and drivers through the parameter; fields of record signals as actuals of
-- scalar out parameters (#215).
library ieee;
use ieee.std_logic_1164.all;

package rsp_pkg is
  type cell is record
    ok : boolean;
    n  : natural;
    w  : std_logic_vector(1 downto 0);
  end record;
  type cell_vec is array (natural range <>) of cell;
  type pair is record
    a : cell;
    b : integer;
  end record;
  function image(c : cell) return string;
end package;

package body rsp_pkg is
  function image(c : cell) return string is
  begin
    return "(" & boolean'image(c.ok) & "," & integer'image(c.n) & ","
      & std_logic'image(c.w(1)) & std_logic'image(c.w(0)) & ")";
  end function;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use work.rsp_pkg.all;

entity record_signal_params is
end entity;

architecture test of record_signal_params is
  signal s : cell_vec(0 to 2) := ((false, 0, "00"), (true, 1, "01"), (false, 2, "10"));
  signal r : cell := (true, 9, "11");
  signal p : pair := ((false, 4, "0Z"), 7);
  signal q : pair := ((false, 4, "00"), 7);

  procedure show(tag : string; signal x : in cell_vec) is
  begin
    report tag & " len " & integer'image(x'length) & " left " & integer'image(x'left)
      & " " & image(x(x'left)) & " " & image(x(x'right))
      & " ev " & boolean'image(x'event) & " lv " & image(x(x'left)'last_value);
  end procedure;

  procedure show1(tag : string; signal x : in cell) is
  begin
    report tag & " " & image(x) & " ev " & boolean'image(x'event)
      & " active " & boolean'image(x'active) & " n'ev " & boolean'image(x.n'event);
  end procedure;

  -- An in parameter passed on to another procedure, whole and in parts.
  procedure relay(signal x : in cell_vec) is
  begin
    show("relay", x);
    show1("relay(1)", x(1));
  end procedure;

  -- A constrained formal takes its own bounds.
  procedure fixed(signal x : in cell_vec(1 to 2)) is
  begin
    report "fixed " & integer'image(x'left) & " " & image(x(1)) & " " & image(x(2));
  end procedure;

  procedure set(signal x : out cell; v : natural) is
  begin
    x <= (v mod 2 = 0, v, "1Z");
  end procedure;

  procedure bump(signal x : inout cell_vec) is
  begin
    x(x'left).n <= x(x'left).n + 10;
    x(x'right) <= (true, x(x'right).n + 1, "11");
  end procedure;

  -- An out parameter passed on to an out parameter, and a field of it driven.
  procedure set_via(signal x : out cell; v : natural) is
  begin
    set(x, v);
  end procedure;

  procedure set_w(signal x : out cell) is
  begin
    x.w <= "ZZ";
  end procedure;

  -- Scalar out parameters with fields of record signals as actuals.
  procedure set_int(signal x : out integer; v : integer) is
  begin
    x <= v;
  end procedure;

  procedure set_bool(signal x : out boolean) is
  begin
    x <= true;
  end procedure;

  -- Waits on the parameter: until any of its leaves changes.
  procedure wait_change(signal x : in cell; tag : string) is
  begin
    wait on x;
    report tag & " changed to " & image(x) & " at " & time'image(now);
  end procedure;
begin
  main : process
  begin
    show("s", s);
    show1("r", r);
    relay(s);
    fixed(s(1 to 2));
    set(r, 4);
    bump(s);
    set_via(p.a, 5);
    set_int(q.b, 11);
    set_bool(q.a.ok);
    set_int(q.a.n, 12);
    wait for 0 ns;
    show1("r", r);
    show("s", s);
    show1("p.a", p.a);
    show1("q.a", q.a);
    report "q.b " & integer'image(q.b);
    assert r = (true, 4, "1Z") severity failure;
    assert s(0).n = 10 and s(2) = (true, 3, "11") severity failure;
    set_w(s(1));
    wait for 1 ns;
    show("s", s);
    show1("s(1)", s(1));
    assert s(1).w = "ZZ" and s(1).n = 1 severity failure;
    wait;
  end process;

  waiter : process
  begin
    wait_change(r, "r");
    wait_change(p.a, "p.a");
    wait;
  end process;
end architecture;
