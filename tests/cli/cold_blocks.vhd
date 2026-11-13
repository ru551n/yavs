-- A check whose failing arm reports and then fails, called through a function and a procedure
-- that both inline it: the report, its call path and the trace of the failure are the ones of
-- the calls as written (`opt/inline.rs` cold blocks).
use std.env.all;
package cold_pkg is
  procedure chk(val, bound : integer);
  impure function wrap(val, bound : integer) return integer;
  procedure outer(val, bound : integer; variable r : out integer);
  type lim_t is record
    lo, hi : integer;
  end record;
  procedure validate_value(lim : lim_t; value : integer);
  impure function twice(lim : lim_t; value : integer) return integer;
end package;
package body cold_pkg is
  type tab_t is array (0 to 3) of integer;
  constant tab : tab_t := (1, 2, 3, 4);
  procedure chk(val, bound : integer) is
  begin
    if val >= bound then
      report "chk " & integer'image(val) & " path " & to_string(get_call_path) severity warning;
      assert tab(val - bound) > 0 report "tab " & integer'image(tab(val - bound)) severity note;
    end if;
  end;
  impure function wrap(val, bound : integer) return integer is
  begin
    chk(val, bound);
    return val + 1;
  end;
  procedure outer(val, bound : integer; variable r : out integer) is
  begin
    r := wrap(val, bound) * 2;
  end;
  -- A record parameter (passed by address), and a function whose check reports and goes on.
  procedure validate_value(lim : lim_t; value : integer) is
  begin
    assert lim.lo <= value and value <= lim.hi
      report "value=" & integer'image(value) & " is out of bounds " & integer'image(lim.lo)
        & " <= value <= " & integer'image(lim.hi) & " path " & to_string(get_call_path);
  end;
  impure function twice(lim : lim_t; value : integer) return integer is
  begin
    assert value < lim.hi report "twice " & integer'image(value) severity warning;
    return 2 * value;
  end;
end package body;

use work.cold_pkg.all;
entity cold_blocks is
  generic (bad : integer := 0);
end;
architecture a of cold_blocks is
begin
  process
    variable acc, r : integer := 0;
    constant lim : lim_t := (lo => 0, hi => 50);
  begin
    for k in 0 to 9 loop
      validate_value(lim, k);
      acc := acc + wrap(k, 100) + twice(lim, k);
    end loop;
    validate_value(lim, 50 + bad);
    validate_value(lim, -bad);
    acc := acc + twice(lim, 50 + bad) + twice(lim, -bad);
    acc := acc + wrap(100 + bad, 100);
    outer(100 + bad, 100, r);
    report "acc=" & integer'image(acc + r);
    wait;
  end process;
end;
