-- difftest: top=named_return_generic_2019 std=2019 ref=hand
-- 2019 named return values (4.2.1) in generic subprograms (of an array type and of a generic
-- type), in a generic package and in protected methods. ref=hand: the reference simulator crashes on the generic
-- subprograms, fails at run time on the methods and rejects a generic type after `of`; the
-- expected output is the LRM result, worked out by hand.
package named_return_gp is
  generic (type value_t);
  function get(x : value_t) return r of value_t;
end package;
package body named_return_gp is
  function get(x : value_t) return r of value_t is
    variable o : r := x;
  begin
    return o;
  end function;
end package body;

entity named_return_generic_2019 is end;
architecture t of named_return_generic_2019 is
  type rec_t is record
    f : integer_vector;
    n : natural;
  end record;

  function fill generic (v : integer) parameter (x : natural) return r of integer_vector is
    variable o : r;
  begin
    for i in o'range loop
      o(i) := v + x + i;
    end loop;
    return o;
  end function;
  function fill3 is new fill generic map (v => 3);

  function zero generic (type t) return r of t is
    variable o : r;
  begin
    return o;
  end function;
  function zbv is new zero generic map (t => bit_vector);
  function zrec is new zero generic map (t => rec_t);
  package gbv is new work.named_return_gp generic map (value_t => bit_vector);

  type store is protected
    procedure set(x : integer);
    impure function get return r of integer_vector;
    impure function getr return r of rec_t;
  end protected;
  type store is protected body
    variable val : integer := 0;
    procedure set(x : integer) is
    begin
      val := x;
    end procedure;
    impure function get return r of integer_vector is
      variable o : r := (others => val);
    begin
      return o;
    end function;
    impure function getr return r of rec_t is
      variable o : r;
    begin
      for i in o.f'range loop
        o.f(i) := val + i;
      end loop;
      o.n := o.f'length;
      return o;
    end function;
  end protected body;
  shared variable sv : store;

  constant k : integer_vector(1 to 2) := fill3(10);
begin
  process
    variable a : integer_vector(1 to 2);
    variable b : bit_vector(5 downto 2) := "1111";
    variable c : rec_t(f(0 to 2));
    variable st : store;
  begin
    report "k " & to_string(k(1)) & " " & to_string(k(2));
    a := fill3(1);
    report "fill3 " & to_string(a(1)) & " " & to_string(a(2));
    b := zbv;
    report "zbv " & to_string(b) & " " & to_string(b'left);
    b := gbv.get("1010");
    report "gbv " & to_string(b) & " " & to_string(b'left);
    c := zrec;
    report "zrec " & to_string(c.f'length) & " " & to_string(c.n);
    sv.set(5);
    a := sv.get;
    report "sv " & to_string(a(1)) & to_string(a(2));
    st.set(7);
    c := st.getr;
    report "st " & to_string(c.f(0)) & to_string(c.f(2)) & " " & to_string(c.n);
    wait;
  end process;
end;
