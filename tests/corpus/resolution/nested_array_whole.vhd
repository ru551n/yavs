-- difftest: top=nested_array_whole std=2008 ref=hand
-- ref=hand: two of the reference simulators give these values; the first resolves the
-- elements with each other's values, the second rejects a process driving part of one.
-- Arrays whose elements are resolved as a whole (LRM 4.6, 14.7.3.1): each element is a basic
-- signal with its own sources. Records (one element per leaf slice), bit vectors (bit-planes)
-- and resolved records inside the records of an unresolved array.
entity nested_array_whole is end;
architecture t of nested_array_whole is
  type rec is record b : bit; n : integer; end record;
  type rec_vec is array (natural range <>) of rec;
  -- b: whether several sources drive the element; n: the sum.
  function res(v : rec_vec) return rec is
    variable r : rec := ('0', 0);
  begin
    if v'length > 1 then r.b := '1'; end if;
    for i in v'range loop r.n := r.n + v(i).n; end loop;
    return r;
  end;
  subtype rrec is res rec;
  type rarr is array (0 to 3) of rrec;
  signal a : rarr := (others => ('0', 1));

  type bv_vec is array (natural range <>) of bit_vector(0 to 3);
  function bor(v : bv_vec) return bit_vector is
    variable r : bit_vector(0 to 3) := "0000";
  begin
    for i in v'range loop r := r or v(i); end loop;
    return r;
  end;
  subtype rbv is bor bit_vector(0 to 3);
  type bv_arr is array (2 downto 0) of rbv;
  signal v : bv_arr := (others => "0000");

  type cell is record id : integer; r : rrec; end record;
  type cells is array (1 to 2) of cell;
  signal c : cells := (others => (0, ('0', 0)));

  function img(r : rec) return string is
  begin
    return bit'image(r.b) & "/" & integer'image(r.n);
  end;
begin
  -- p3 drives the whole array (a loop index is not static). a(0): p1, p3; a(1): p1, p2 (only
  -- n), p3; a(2): p2, p3; a(3): p3 alone.
  p1 : process begin
    a(0) <= ('0', 3) after 1 ns;
    a(1).b <= '1' after 1 ns;
    wait;
  end process;
  p2 : process begin
    wait for 2 ns;
    a(1).n <= 20;
    a(2) <= ('1', 30);
    wait;
  end process;
  p3 : process begin
    wait for 3 ns;
    for i in a'range loop
      a(i).n <= 100 * i;
    end loop;
    wait;
  end process;
  -- v(2): q1 and q2; v(1): q2; v(0): no source.
  q1 : process begin
    v(2) <= "1000" after 1 ns;
    wait;
  end process;
  q2 : process begin
    v(2)(3) <= '1' after 2 ns;
    v(1) <= "0110" after 2 ns;
    wait;
  end process;
  -- c(1).r: r1 and r2; c(2).r: r2; c(2).id: r1.
  r1 : process begin
    c(1).r.n <= 4 after 1 ns;
    c(2).id <= 9 after 1 ns;
    wait;
  end process;
  r2 : process begin
    c(1).r <= ('1', 6) after 2 ns;
    c(2).r.n <= 8 after 2 ns;
    wait;
  end process;
  process (a) begin
    report "a " & img(a(0)) & " " & img(a(1)) & " " & img(a(2)) & " " & img(a(3));
  end process;
  process (v) begin
    report "v " & to_string(v(2)) & " " & to_string(v(1)) & " " & to_string(v(0));
  end process;
  process (c) begin
    report "c " & integer'image(c(1).id) & " " & img(c(1).r) & " "
      & integer'image(c(2).id) & " " & img(c(2).r);
  end process;
  process begin
    wait for 5 ns;
    assert a = (('1', 3), ('1', 121), ('1', 230), ('0', 300)) severity failure;
    assert v = ("1001", "0110", "0000") severity failure;
    assert c = ((0, ('1', 10)), (9, ('0', 8))) severity failure;
    report "done";
    wait;
  end process;
end;
