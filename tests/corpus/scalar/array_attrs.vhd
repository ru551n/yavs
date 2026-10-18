-- difftest: top=array_attrs std=2008
entity array_attrs is end;
architecture t of array_attrs is
  type mem is array (3 downto 0, 1 to 5) of integer;
  type vec is array (natural range <>) of bit;
  signal s : bit_vector(7 downto 2);
begin
  process
    variable m : mem;
    variable v : vec(10 to 14);
  begin
    assert m'length(1) = 4 and m'length(2) = 5 and m'left(1) = 3 and m'right(2) = 5 severity failure;
    assert m'low(1) = 0 and m'high(1) = 3 and not m'ascending(1) and m'ascending(2) severity failure;
    assert v'length = 5 and v'low = 10 and v'high = 14 severity failure;
    assert s'length = 6 and s'left = 7 and s'right = 2 and not s'ascending severity failure;
    for i in s'reverse_range loop report "ri " & integer'image(i); end loop;
    for i in v'range loop v(i) := '1'; end loop;
    report "len=" & integer'image(v'length) & " sub=" & integer'image(v(11 to 12)'length);
    wait;
  end process;
end;
