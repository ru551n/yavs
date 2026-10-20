-- difftest: top=layout_memo_lengths std=2008
-- Variables and results sized by the bounds of unconstrained parameters, called with changing
-- lengths in turn (string, integer array, real array, logic vector, a record holding a string),
-- including null arrays and descending ranges (#444: the layout of a type is memoised per
-- descriptor and must follow every change of bounds).
entity layout_memo_lengths is
end entity;

architecture a of layout_memo_lengths is
  type int_vec is array (natural range <>) of integer;
  type real_vec is array (natural range <>) of real;
  type rec is record
    n    : natural;
    name : string(1 to 6);
  end record;

  function sum_str (s : string) return integer is
    variable v : string(1 to s'length) := s;
    variable t : integer := 0;
  begin
    for i in v'range loop
      t := t + character'pos(v(i)) * (i mod 7 + 1);
    end loop;
    return t + v'length * 1000;
  end function;

  function sum_int (a : int_vec) return integer is
    variable v : int_vec(a'length - 1 downto 0) := a;
    variable t : integer := 0;
  begin
    for i in v'range loop
      t := t + v(i) * (i + 1);
    end loop;
    return t + v'length * 1000;
  end function;

  function sum_real (a : real_vec) return integer is
    variable v : real_vec(1 to a'length) := a;
    variable t : real := 0.0;
  begin
    for i in v'range loop
      t := t + v(i) * real(i);
    end loop;
    return integer(t) + v'length * 1000;
  end function;

  function sum_slv (a : bit_vector) return integer is
    variable v : bit_vector(a'length - 1 downto 0) := a;
    variable t : integer := 0;
  begin
    for i in v'range loop
      if v(i) = '1' then
        t := t + 2 ** (i mod 16);
      end if;
    end loop;
    return t + v'length * 100000;
  end function;

  function twice (s : string) return string is
    variable r : string(1 to 2 * s'length);
  begin
    r(1 to s'length) := s;
    r(s'length + 1 to r'length) := s;
    return r;
  end function;

  function name_len (r : rec) return integer is
  begin
    return r.n + r.name'length;
  end function;

  constant S1 : string := "a";
  constant S2 : string := "hello, world";
  constant S3 : string := "";
  constant S4 : string(5 downto 1) := "abcde";
  constant I1 : int_vec := (3, 1, 4, 1, 5, 9, 2, 6);
  constant I2 : int_vec(10 to 11) := (7, 8);
  constant I3 : int_vec(1 to 0) := (others => 0);
  constant R1 : real_vec := (1.0, 2.0, 3.0);
  constant R2 : real_vec := (0.5, 0.25, 4.0, 8.0, 16.0);
  constant B1 : bit_vector := "10110";
  constant B2 : bit_vector := "1111000011110000111100001";
  constant B3 : bit_vector(0 downto 1) := (others => '0');
begin
  process
    variable r : rec := (n => 4, name => "abcdef");
  begin
    for k in 1 to 3 loop
      assert sum_str(S1) = sum_str("a") report "S1" severity failure;
      report "str " & integer'image(sum_str(S1)) & " " & integer'image(sum_str(S2)) & " "
        & integer'image(sum_str(S3)) & " " & integer'image(sum_str(S4));
      report "int " & integer'image(sum_int(I1)) & " " & integer'image(sum_int(I2)) & " "
        & integer'image(sum_int(I3));
      report "real " & integer'image(sum_real(R1)) & " " & integer'image(sum_real(R2));
      report "bits " & integer'image(sum_slv(B1)) & " " & integer'image(sum_slv(B2)) & " "
        & integer'image(sum_slv(B3));
      report twice(S1) & "|" & twice(S2) & "|" & twice(S3) & "|" & twice(S4);
      r.n := k;
      report "rec " & integer'image(name_len(r));
    end loop;
    wait;
  end process;
end architecture;
