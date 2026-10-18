-- difftest: top=arrays_aggregates std=2008
entity arrays_aggregates is end;
architecture t of arrays_aggregates is
  type int_vec is array (natural range <>) of integer;
  type matrix is array (1 to 2, 1 to 3) of integer;
  constant c : int_vec(0 to 4) := (0 => 5, 1 to 2 => 7, others => -1);
  function sum(v : int_vec) return integer is variable s : integer := 0; begin
    for i in v'range loop s := s + v(i); end loop; return s;
  end;
begin
  process
    variable m : matrix := ((1, 2, 3), (4, 5, 6));
    variable v : int_vec(1 to 6);
    variable w : int_vec(0 to 9);
  begin
    report "sum c=" & integer'image(sum(c));
    for i in 1 to 2 loop for j in 1 to 3 loop v((i - 1) * 3 + j) := m(i, j) * 10; end loop; end loop;
    report "sum v=" & integer'image(sum(v)) & " slice=" & integer'image(sum(v(2 to 4)));
    v(1 to 3) := v(4 to 6);
    report "v1=" & integer'image(v(1)) & " v3=" & integer'image(v(3));
    v(2 to 4) := v(1 to 3);
    report "overlap v2=" & integer'image(v(2)) & " v4=" & integer'image(v(4));
    w := c & c;
    assert w'length = 10 and w(7) = 7 severity failure;
    assert c(1 to 2) = (7, 7) severity failure;
    wait;
  end process;
end;
