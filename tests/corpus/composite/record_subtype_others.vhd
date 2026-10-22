-- difftest: top=tb_record_subtype_others std=2008
-- A variable of the subtype of a record parameter (`r'subtype`) is constrained by that
-- parameter's actual: `others` in its aggregate takes the bounds of the element of the same
-- name, also for a nested record.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

package pk_rso is
  type inner_t is record
    tag  : std_ulogic;
    bits : std_ulogic_vector;
  end record;
  type rec_t is record
    sign     : std_ulogic;
    mantissa : unsigned;
    inner    : inner_t;
  end record;
  function make(s : std_ulogic; r : rec_t) return rec_t;
end package;

package body pk_rso is
  function make(s : std_ulogic; r : rec_t) return rec_t is
    variable v : r'subtype := (
      sign => s,
      mantissa => (others => '1'),
      inner => (tag => not s, bits => (others => s))
    );
  begin
    return v;
  end function;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.pk_rso.all;

entity tb_record_subtype_others is end;
architecture t of tb_record_subtype_others is
begin
  process
    variable a : rec_t(mantissa(7 downto 3), inner(bits(2 to 4)));
    variable b : rec_t(mantissa(1 downto 0), inner(bits(5 downto 1)));
    variable x, y : rec_t(mantissa(7 downto 3), inner(bits(2 to 4)));
  begin
    x := make('1', a);
    assert x.mantissa'length = 5 and x.mantissa = "11111" report "mantissa" severity failure;
    assert x.inner.bits'left = 2 and x.inner.bits = "111" and x.inner.tag = '0'
      report "inner" severity failure;
    assert x.sign = '1' report "sign" severity failure;
    y := make('0', a);
    assert y.inner.bits = "000" and y.inner.tag = '1' report "inner 0" severity failure;
    report integer'image(make('1', b).mantissa'length) & " " & integer'image(make('1', b).inner.bits'length);
    wait;
  end process;
end;
