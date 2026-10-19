-- (from the differential corpus)
-- ref=hand: signal formals with records associated field by field, each field (or element of
-- a field, or nested record) in another signal (LRM 6.5.7.1, #71): reads, 'event, drives
-- of the whole formal and of fields. The first reference simulator does not support this
-- form of parameter; .expected is hand-written, the second reference simulator gives the
-- same values and the asserts check them.
library ieee;
use ieee.std_logic_1164.all;
entity signal_param_records is end;
architecture t of signal_param_records is
  type rec is record
    a : integer;
    v : std_logic_vector(1 downto 0);
    b : bit;
  end record;
  signal n : integer := 7;
  signal v1, v0 : std_logic := '0';
  signal b : bit := '1';
  signal r2 : rec := (1, "01", '0');
  type rec2 is record
    s : rec;
    c : integer;
  end record;

  procedure show(signal x : in rec) is
  begin
    report "show " & integer'image(x.a) & " " & to_string(x.v) & " " & bit'image(x.b)
      & " event=" & boolean'image(x'event);
  end procedure;

  procedure set(signal x : out rec; a : integer) is
  begin
    x <= (a, "10", '0');
  end procedure;

  procedure show2(signal y : in rec2) is
  begin
    show(y.s);
    report "c " & integer'image(y.c);
  end procedure;

  procedure clear(signal y : out rec2) is
  begin
    y.s.a <= 0;
    y.c <= -1;
  end procedure;

  procedure bump(signal x : inout rec) is
  begin
    x.a <= x.a + 1;
    x.v(0) <= not x.v(0);
  end procedure;
begin
  process
  begin
    show(x.a => n, x.v(1) => v1, x.v(0) => v0, x.b => b);
    set(x.v(0) => v0, x.b => b, x.a => n, x.v(1) => v1, a => 42);
    wait for 0 ns;
    show(x.a => n, x.v(1) => v1, x.v(0) => v0, x.b => b);
    assert n = 42 and v1 = '1' and v0 = '0' and b = '0';
    bump(x.a => r2.a, x.v => r2.v, x.b => b);
    wait for 0 ns;
    show(x.a => r2.a, x.v => r2.v, x.b => b);
    assert r2.a = 2 and r2.v = "00";
    show2(y.s => r2, y.c => n);
    clear(y.c => n, y.s.v => r2.v, y.s.a => r2.a, y.s.b => b);
    wait for 0 ns;
    show2(y.s => r2, y.c => n);
    assert r2.a = 0 and n = -1;
    wait;
  end process;
end architecture;
