-- difftest: top=aggregate_fill std=2008
-- Aggregates and defaults of long arrays whose elements are all equal: range and `others`
-- choices (ascending and descending, empty, dynamic), arrays of access values, reals, records
-- and arrays of arrays (#405).
entity aggregate_fill is end;
architecture t of aggregate_fill is
  type int_a is array (natural range <>) of integer;
  type up_t is array (0 to 9) of integer;
  type dn_t is array (9 downto 0) of integer;
  type rec_t is record a : integer; b : boolean; c : character; end record;
  type rec_a is array (natural range <>) of rec_t;
  type ptr_t is access integer;
  type ptr_a is array (natural range <>) of ptr_t;
  type pa_t is access ptr_a;
  type real_a is array (natural range <>) of real;
  type row_t is array (0 to 2) of integer;
  type rows_t is array (natural range <>) of row_t;
  constant k : integer := 5;
  function sum(v : int_a) return integer is
    variable s : integer := 0;
  begin
    for i in v'range loop s := s + v(i) * (i + 1); end loop;
    return s;
  end;
begin
  process
    variable u : up_t := (others => 7);
    variable d : dn_t := (3 downto 1 => 4, 8 to 9 => 6, others => -1);
    variable d2 : dn_t := (0 to 2 => 4, 9 downto 8 => 2, others => 0);
    variable u2 : up_t := (2 to 4 => k, 7 downto 5 => 11, others => 0);
    variable r : rec_a(0 to 99);
    variable r2 : rec_a(1 to 0);
    variable rows : rows_t(0 to 4);
    variable p : pa_t;
    variable x : real_a(0 to 4) := (1 to 3 => 2.5, others => 0.5);
    variable lo, hi : integer;
    variable e : int_a(1 to 0) := (others => 3);
    variable s : string(1 to 6) := (2 to 4 => 'x', others => '.');
    variable bs : bit_vector(0 to 7) := (3 to 5 => '1', others => '0');
    variable big : pa_t;
  begin
    assert sum(int_a(u)) = 7 * 55;
    report "d " & integer'image(sum(int_a(d))) & " d2 " & integer'image(sum(int_a(d2)))
      & " u2 " & integer'image(sum(int_a(u2)));
    assert r(99).a = integer'left and not r(50).b and r(7).c = character'left and r'length = 100
      and r2'length = 0;
    assert rows(4)(2) = integer'left and rows'length = 5;
    p := new ptr_a'(0 to 5 => null);
    assert p(5) = null and p'length = 6;
    p := new ptr_a'(0 to 5 => new integer'(3));
    assert p(5).all = 3;
    assert x(0) = 0.5 and x(1) = 2.5 and x(3) = 2.5 and x(4) = 0.5;
    assert e'length = 0 and s = ".xxx.." and bs = "00011100";
    lo := 2; hi := 6;
    p := new ptr_a'(lo to hi => null);
    assert p'left = 2 and p'right = 6 and p(4) = null;
    lo := 6; hi := 2;
    p := new ptr_a'(lo to hi => null);
    assert p'length = 0;
    lo := 6; hi := 3;
    p := new ptr_a'(lo downto hi => null);
    assert p'length = 4 and p(3) = null and p(6) = null;
    -- a long array: the whole of it is written
    big := new ptr_a'(0 to 70000 => null);
    assert big(70000) = null and big'length = 70001;
    big := new ptr_a'(0 to 70000 => new integer'(9));
    assert big(0).all = 9 and big(70000).all = 9;
    report "done";
    wait;
  end process;
end;
