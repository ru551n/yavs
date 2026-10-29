-- difftest: top=range_attr_attrs std=2008
-- `'left`, `'right`, `'low`, `'high` and `'ascending` of a range attribute read the
-- same attribute of its prefix (#818), also of a dimension of a two-dimensional array (#613).
entity range_attr_attrs is
end entity;
architecture a of range_attr_attrs is
  type my_type is array (0 to 3) of integer;
  type m is array (1 to 4, 7 downto 2) of bit;
  signal m1 : m;
  procedure show (input : bit_vector) is
  begin
    report "input " & integer'image(input'range'left) & " " & integer'image(input'range'right);
  end procedure;
begin
  process
    variable m : bit_vector(7 downto 2);
  begin
    assert my_type'range'left = 0 severity failure;
    assert my_type'range'right = 3 severity failure;
    assert my_type'range'high = 3 and my_type'range'low = 0 severity failure;
    assert my_type'range'ascending severity failure;
    assert m1'range(2)'low = 2 and m1'range(2)'high = 7 severity failure;
    assert m1'range(2)'left = 7 and m1'range(2)'right = 2 severity failure;
    assert m1'range(1)'high = 4 and m1'range(1)'low = 1 severity failure;
    assert m1'range(1)'ascending and not m1'range(2)'ascending severity failure;
    show(m);
    show("101");
    for i in m'range'left downto m'range'right loop
      report "i " & integer'image(i);
    end loop;
    wait;
  end process;
end architecture;
