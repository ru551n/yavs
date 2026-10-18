-- difftest: top=enum_attrs std=2008
entity enum_attrs is end;
architecture t of enum_attrs is
  type color is (red, green, blue, 'x', violet);
  subtype warm is color range red to green;
begin
  process
    variable c : color := green;
  begin
    assert color'left = red and color'right = violet and color'low = red severity failure;
    assert color'pos(blue) = 2 and color'val(1) = green severity failure;
    assert color'succ(red) = green and color'pred(blue) = green severity failure;
    assert warm'high = green and warm'rightof(red) = green severity failure;
    assert color'leftof(green) = red severity failure;
    report "image=" & color'image(c) & " " & color'image('x') & " " & color'image(violet);
    report "value=" & color'image(color'value("BLUE"));
    assert boolean'image(true) = "true" and boolean'pos(true) = 1 severity failure;
    assert character'pos('A') = 65 and character'val(97) = 'a' severity failure;
    report "bit=" & bit'image('1') & " sev=" & severity_level'image(warning);
    for i in color loop report integer'image(color'pos(i)) & ":" & color'image(i); end loop;
    wait;
  end process;
end;
