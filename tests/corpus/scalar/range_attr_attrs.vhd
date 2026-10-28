-- difftest: top=range_attr_attrs std=2008
-- `'left`, `'right`, `'low`, `'high` and `'ascending` of a range attribute read the
-- same attribute of its prefix (#818).
entity range_attr_attrs is
end entity;
architecture a of range_attr_attrs is
  type my_type is array (0 to 3) of integer;
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
    show(m);
    show("101");
    for i in m'range'left downto m'range'right loop
      report "i " & integer'image(i);
    end loop;
    wait;
  end process;
end architecture;
