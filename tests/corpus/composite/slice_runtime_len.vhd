-- difftest: top=slice_runtime_len std=2008
-- Slices whose length is known only at run time, copied into a variable at a run-time offset:
-- bounds from generics in a function (a slice of an unconstrained parameter returned), from
-- parameters, null slices, 1 to 64 elements around block edges and over 64, descending and
-- ascending actuals, a slice as the actual, a function with a loop over its parameter, and the
-- same variable as source and target. Each copy is checked against an element loop.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity slice_runtime_len is
  generic (g_hi : integer := 13; g_lo : integer := 3);
end;

architecture t of slice_runtime_len is
  subtype row_t is unsigned(g_hi - 1 - g_lo downto 0);
  constant codes : std_ulogic_vector(0 to 8) := "UX01ZWLH-";

  function get_row(addr : std_ulogic_vector) return row_t is
  begin
    return unsigned(addr(g_hi - 1 downto g_lo));
  end;

  function down(v : std_ulogic_vector; hi, lo : integer) return std_ulogic_vector is
  begin
    return v(hi downto lo);
  end;

  function up(v : std_ulogic_vector; lo, hi : integer) return std_ulogic_vector is
  begin
    return v(lo to hi);
  end;

  function reverse(a : std_ulogic_vector) return std_ulogic_vector is
    variable r : std_ulogic_vector(a'range);
  begin
    for i in a'range loop
      r(a'left + a'right - i) := a(i);
    end loop;
    return r;
  end;

  function pattern(n, seed : natural) return std_ulogic_vector is
    variable v : std_ulogic_vector(n - 1 downto 0);
  begin
    for i in v'range loop
      v(i) := codes((i * i + i * (2 * seed + 1) + seed) mod 9);
    end loop;
    return v;
  end;

  type pair is record hi, lo : integer; end record;
  type pairs is array (natural range <>) of pair;
  -- hi < lo: a null slice.
  constant shapes : pairs := (
    (0, 1), (5, 6), (64, 65), (0, 0), (63, 63), (64, 64), (1, 0), (63, 0), (64, 1), (65, 2),
    (31, 0), (32, 1), (95, 32), (126, 63), (127, 64), (127, 65), (64, 0), (100, 1), (127, 0),
    (70, 63));
begin
  process
    variable a : std_ulogic_vector(127 downto 0) := pattern(128, 1);
    variable u : std_ulogic_vector(0 to 127);
    variable r, w : std_ulogic_vector(127 downto 0);
    variable x : std_ulogic_vector(127 downto 0);
    variable row : row_t;
    variable off, n, sum : integer := 0;
  begin
    for i in u'range loop
      u(i) := a(127 - i);
    end loop;
    -- A slice of generic bounds out of a function, from actuals of other bounds.
    row := get_row(a(63 downto 0));
    report "row " & to_string(std_ulogic_vector(row));
    row := get_row(a);
    report "row128 " & to_string(std_ulogic_vector(row));
    row := get_row(a(100 downto 2));
    report "row slice " & to_string(std_ulogic_vector(row));
    for k in shapes'range loop
      n := shapes(k).hi - shapes(k).lo + 1;
      if n < 0 then
        n := 0;
      end if;
      for o in 0 to 2 loop
        off := (o * 61 + k) mod (129 - n);
        r := pattern(128, 2);
        w := r;
        -- Run-time count at a run-time offset, from a descending and an ascending actual.
        r(off + n - 1 downto off) := down(a, shapes(k).hi, shapes(k).lo);
        for i in 0 to n - 1 loop
          w(off + i) := a(shapes(k).lo + i);
        end loop;
        assert r = w report "down " & integer'image(k) & " " & integer'image(o) severity failure;
        r(off + n - 1 downto off) := up(u, 127 - shapes(k).hi, 127 - shapes(k).lo);
        assert r = w report "up " & integer'image(k) & " " & integer'image(o) severity failure;
        -- A slice of a slice actual.
        if shapes(k).hi <= 110 then
          r(off + n - 1 downto off) := down(a(120 downto 0), shapes(k).hi, shapes(k).lo);
          assert r = w report "slice " & integer'image(k) severity failure;
        end if;
        -- In place: the same variable as source and target.
        x := a;
        x(off + n - 1 downto off) := x(shapes(k).hi downto shapes(k).lo);
        w := a;
        for i in 0 to n - 1 loop
          w(off + i) := a(shapes(k).lo + i);
        end loop;
        assert x = w report "self " & integer'image(k) & " " & integer'image(o) severity failure;
        sum := sum + n;
      end loop;
      report "shape " & integer'image(k) & " " & to_string(r(off + n - 1 downto off)) & " "
        & to_string(x(70 downto 0));
    end loop;
    -- A loop over an unconstrained parameter: both directions, null, over 64 elements.
    report "rev " & to_string(reverse(a(9 downto 0))) & " " & to_string(reverse(u(3 to 12)));
    report "rev null " & integer'image(reverse(a(3 downto 4))'length);
    report "rev wide " & to_string(reverse(a(99 downto 20)));
    report "sum " & integer'image(sum);
    wait;
  end process;
end;
