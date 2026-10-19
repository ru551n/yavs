-- LRM 2019 4.2.1: named return values of a record type, of a generic subprogram (also of a
-- generic type) and of a protected method; each call takes the target's subtype.
entity top is end entity;
architecture a of top is
  type rec_t is record
    f : integer_vector;
    n : natural;
  end record;
  function iota return r of rec_t is
    variable v : r;
  begin
    for i in v.f'range loop v.f(i) := i; end loop;
    v.n := v.f'length;
    return v;
  end function;
  function fill generic (k : integer) parameter (x : natural) return r of integer_vector is
    variable o : r := (others => k + x);
  begin
    return o;
  end function;
  function fill3 is new fill generic map (k => 3);
  function zero generic (type t) return r of t is
    variable o : r;
  begin
    return o;
  end function;
  function zbv is new zero generic map (t => bit_vector);
  type store is protected
    impure function get return r of integer_vector;
  end protected;
  type store is protected body
    variable val : integer := 5;
    impure function get return r of integer_vector is
      variable o : r := (others => val);
    begin
      return o;
    end function;
  end protected body;
begin
  process
    variable c : rec_t(f(2 to 4));
    variable a : integer_vector(1 to 2);
    variable b : bit_vector(3 downto 0) := "1111";
    variable s : store;
  begin
    c := iota;
    report "RES record " & integer'image(c.f(4)) & " " & integer'image(c.n);
    a := fill3(1);
    report "RES generic " & integer'image(a(2));
    b := zbv;
    report "RES generic type " & integer'image(b'length) & " " & bit'image(b(3));
    a := s.get;
    report "RES method " & integer'image(a(1));
    wait;
  end process;
end architecture;
