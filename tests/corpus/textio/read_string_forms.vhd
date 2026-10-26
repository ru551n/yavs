-- difftest: top=read_string_forms std=2008
-- READ of a string target of any bounds, direction and length (the target arrives as data and
-- bounds, #771): whitespace is part of the value, a short line fails with GOOD false and stays as
-- it was, and strings above 64 characters take the long path.
use std.textio.all;
entity read_string_forms is end;
architecture a of read_string_forms is
begin
  process
    variable l : line;
    variable up : string(1 to 4);
    variable down : string(7 downto 4);
    variable off : string(10 to 12);
    variable none : string(1 to 0);
    variable long : string(1 to 70);
    variable good : boolean;
    variable text : string(1 to 70);
  begin
    l := new string'("ab d efgh");
    read(l, up);
    assert up = "ab d" report "up: " & up severity failure;
    assert l.all = " efgh" report "rest: " & l.all severity failure;
    read(l, down, good);
    assert good and down = " efg" and down'left = 7 report "down: " & down severity failure;
    assert l.all = "h" report "rest 2: " & l.all severity failure;
    read(l, off, good);
    assert not good report "off must fail" severity failure;
    assert l.all = "h" report "short line changed: " & l.all severity failure;
    read(l, none, good);
    assert good and l.all = "h" report "null string" severity failure;
    read(l, off(10 to 10), good);
    assert good and off(10) = 'h' and l.all = "" report "slice target" severity failure;
    for i in text'range loop
      text(i) := character'val(97 + i mod 26);
    end loop;
    l := new string'(text & "tail");
    read(l, long, good);
    assert good and long = text and l.all = "tail" report "long" severity failure;
    read(l, long, good);
    assert not good and l.all = "tail" report "long short" severity failure;
    report "done";
    wait;
  end process;
end;
