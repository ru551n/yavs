-- difftest: top=int_attrs std=2008
entity int_attrs is end;
architecture t of int_attrs is
  subtype small is integer range -5 to 10;
  type byte is range 0 to 255;
begin
  process begin
    assert small'low = -5 and small'high = 10 and small'left = -5 and small'right = 10 severity failure;
    assert small'ascending and byte'high = 255 severity failure;
    assert small'pos(3) = 3 and small'val(4) = 4 severity failure;
    assert small'succ(3) = 4 and small'pred(3) = 2 severity failure;
    assert integer'high = 2147483647 and integer'low = -2147483648 severity failure;
    report "small'image=" & small'image(-3) & " byte'image=" & byte'image(200);
    report "value=" & integer'image(integer'value(" 42 ")) & " " & integer'image(small'value("-5"));
    assert natural'low = 0 and positive'low = 1 severity failure;
    wait;
  end process;
end;
