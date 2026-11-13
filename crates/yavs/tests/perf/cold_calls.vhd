-- Library-style accessors: a package wrapper validates its arguments (an assertion with a built
-- message) and calls a protected storage method; a queue pushes and pops through two storages.
-- The failing path of each check is a cold block (`opt/inline.rs`): the rest is inlined.
package cold_pkg is
  type store_t is protected
    impure function get(i : natural) return integer;
    procedure set(i : natural; v : integer);
  end protected;
  type arr_t is record
    len : natural;
    lo, hi : integer;
  end record;
  procedure validate_bounds(name : string; val, bound : integer);
  procedure validate_value(arr : arr_t; value : integer);
  impure function get(arr : arr_t; idx : integer) return integer;
  procedure set(arr : arr_t; idx : integer; value : integer);
  procedure push(q : integer; v : integer);
  impure function pop(q : integer) return integer;
end package;
package body cold_pkg is
  type store_t is protected body
    type iv is array (natural range <>) of integer;
    type iv_ptr is access iv;
    variable d : iv_ptr := new iv'(0 to 255 => 0);
    impure function get(i : natural) return integer is
    begin
      return d(i);
    end function;
    procedure set(i : natural; v : integer) is
    begin
      d(i) := v;
    end procedure;
  end protected body;
  shared variable st, meta, qd : store_t;
  procedure validate_bounds(name : string; val, bound : integer) is
  begin
    assert 0 <= val and val < bound
      report name & "=" & integer'image(val) & " is out of bounds 0 <= " & name & " < "
        & integer'image(bound);
  end;
  procedure validate_value(arr : arr_t; value : integer) is
  begin
    assert arr.lo <= value and value <= arr.hi
      report "value=" & integer'image(value) & " is out of bounds " & integer'image(arr.lo)
        & " <= value <= " & integer'image(arr.hi);
  end;
  impure function get(arr : arr_t; idx : integer) return integer is
  begin
    validate_bounds("idx", idx, arr.len);
    return st.get(idx);
  end;
  procedure set(arr : arr_t; idx : integer; value : integer) is
  begin
    validate_bounds("idx", idx, arr.len);
    validate_value(arr, value);
    st.set(idx, value);
  end;
  procedure push(q : integer; v : integer) is
    variable tail : integer;
  begin
    assert q /= 0 report "Push to null queue";
    tail := meta.get(2 * q);
    qd.set(tail mod 256, v);
    meta.set(2 * q, tail + 1);
  end;
  impure function pop(q : integer) return integer is
    variable head : integer;
  begin
    assert q /= 0 report "Pop from null queue";
    head := meta.get(2 * q + 1);
    assert meta.get(2 * q) > head report "Pop from empty queue";
    meta.set(2 * q + 1, head + 1);
    return qd.get(head mod 256);
  end;
end package body;

use work.cold_pkg.all;
entity cold_calls is end;
architecture a of cold_calls is
begin
  process
    constant arr : arr_t := (len => 256, lo => -5, hi => 100000);
    variable acc : integer := 0;
  begin
    for k in 0 to 99 loop
      set(arr, k, k * 3);
      acc := acc + get(arr, (k * 7) mod 100);
      push(1, k);
      acc := acc + pop(1);
    end loop;
    report "acc=" & integer'image(acc);
    wait;
  end process;
end;
