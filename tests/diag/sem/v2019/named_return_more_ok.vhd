-- yavs-test: std=2019
-- expect: none
-- LRM 2019 4.2.1: named return values of record types, in generic subprograms (also of a
-- generic type, also of a generic package) and in protected methods
package gp is
  generic (type value_t);
  function get(x : value_t) return r of value_t;
end package;
package body gp is
  function get(x : value_t) return r of value_t is
    variable o : r := x;
  begin
    return o;
  end function;
end package body;
package p is
  type rec_t is record
    f : integer_vector;
    n : natural;
  end record;
  function iota return r of rec_t;
  function zero generic (type t) return r of t;
  type store is protected
    impure function get return r of integer_vector;
  end protected;
end package;
package body p is
  function iota return r of rec_t is
    variable v : r;
  begin
    for i in v.f'range loop
      v.f(i) := i;
    end loop;
    v.n := v.f'length;
    return v;
  end function;
  function zero generic (type t) return r of t is
    variable o : r;
  begin
    return o;
  end function;
  function zv is new zero generic map (t => bit_vector);
  function zr is new zero generic map (t => rec_t);
  package gbv is new work.gp generic map (value_t => bit_vector);
  type store is protected body
    variable val : integer := 0;
    impure function get return r of integer_vector is
      variable o : r := (others => val);
    begin
      return o;
    end function;
  end protected body;
  procedure use_them is
    variable s : store;
    variable a : integer_vector(1 to 2) := s.get;
    variable b : bit_vector(3 downto 0) := zv;
    variable c : rec_t(f(0 to 1)) := zr;
  begin
    c := iota;
    b := gbv.get("1010");
  end procedure;
end package body;
