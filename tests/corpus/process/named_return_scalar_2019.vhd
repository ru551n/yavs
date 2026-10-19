-- difftest: top=named_return_scalar_2019 std=2019 ref=hand
-- 2019 named return values (4.2.1) of scalar types: `return r of t` makes `r` the call's
-- target subtype, so `r` has the target's range (integer, enumeration, physical, floating
-- point, a type mark with a range constraint), also in a generic subprogram whose generic
-- type is scalar, in a generic package and in a protected method. ref=hand: the reference
-- simulator rejects scalar named return values; the output is that of the reference
-- simulators that give `r` the target's range (one other takes the type mark's range).
package named_return_scalar_gp is
  generic (type value_t);
  function get(x : value_t) return r of value_t;
end package;
package body named_return_scalar_gp is
  function get(x : value_t) return r of value_t is
    variable o : r := x;
  begin
    return o;
  end function;
end package body;

entity named_return_scalar_2019 is end;
architecture t of named_return_scalar_2019 is
  subtype small is integer range 3 to 9;
  subtype down is integer range 20 downto 10;
  type color is (red, green, blue, white);
  subtype rgb is color range green to white;
  type len is range 0 to 1000 units mm; cm = 10 mm; end units;
  subtype short is len range 5 mm to 50 mm;
  subtype unit_r is real range -1.0 to 1.0;

  function hi return r of integer is
    variable v : r;
  begin
    v := r'high;
    return v;
  end function;
  function lft return r of integer is
  begin
    return r'left;
  end function;
  function asc return r of integer is
  begin
    if r'ascending then
      return r'low + 1;
    end if;
    return r'high - 1;
  end function;
  function last return r of color is
    variable v : r := r'right;
  begin
    return v;
  end function;
  function maxl return r of len is
  begin
    return r'high;
  end function;
  function lo_r return r of real is
  begin
    return r'low;
  end function;
  function cond(c : boolean) return r of integer is
  begin
    return r'low when c;
    return r'high;
  end function;
  function mid return r of small is
  begin
    return (r'low + r'high) / 2;
  end function;
  function span_of generic (type t is (<>)) return r of t is
  begin
    return r'high;
  end function;
  function cspan is new span_of generic map (t => color);
  function pass generic (type t) parameter (x : t) return r of t is
  begin
    return x;
  end function;
  function pi is new pass generic map (t => integer);
  package gi is new work.named_return_scalar_gp generic map (value_t => integer);

  type store is protected
    procedure set(x : integer);
    impure function get return r of integer;
  end protected;
  type store is protected body
    variable val : integer := 0;
    procedure set(x : integer) is
    begin
      val := x;
    end procedure;
    impure function get return r of integer is
    begin
      if val > r'high then
        return r'high;
      end if;
      return val;
    end function;
  end protected body;

  constant k : small := hi;
  signal s : down := asc;
begin
  process
    variable v : small;
    variable d : down;
    variable c : rgb;
    variable l : short;
    variable x : unit_r;
    variable st : store;
  begin
    report "k " & integer'image(k) & " s " & integer'image(s);
    v := hi;
    report "hi " & integer'image(v);
    d := hi;
    report "hi down " & integer'image(d);
    d := lft;
    report "left down " & integer'image(d);
    d := asc;
    report "asc down " & integer'image(d);
    c := last;
    report "last " & color'image(c);
    c := cspan;
    report "cspan " & color'image(c);
    l := maxl;
    report "maxl " & len'image(l);
    x := lo_r;
    report "lo_r " & integer'image(integer(x * 1000.0));
    v := cond(true);
    report "cond true " & integer'image(v);
    d := cond(false);
    report "cond false " & integer'image(d);
    v := mid;
    report "mid " & integer'image(v);
    d := pi(15);
    report "pi " & integer'image(d);
    d := gi.get(12);
    report "gi " & integer'image(d);
    st.set(100);
    v := st.get;
    report "st " & integer'image(v);
    wait;
  end process;
end;
