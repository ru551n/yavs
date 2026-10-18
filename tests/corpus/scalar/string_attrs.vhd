-- difftest: top=string_attrs std=2008
entity string_attrs is end;
architecture t of string_attrs is
begin
  process
    variable s : string(1 to 5) := "hello";
  begin
    assert s'length = 5 and s(1) = 'h' and s(2 to 3) = "el" severity failure;
    assert s & "!" = "hello!" and string'("a") < "b" and string'("abc") < "abd" severity failure;
    report s & " " & character'image(s(1)) & " " & integer'image(character'pos(s(5)));
    assert to_string(42) = "42" and to_string(true) = "true" severity failure;
    assert to_string(character'('x')) = "x" severity failure;
    report "high=" & integer'image(s'high);
    report "bv=" & to_string(bit_vector'(B"1010")) & " hex=" & to_hstring(bit_vector'(X"A5"));
    report "ostr=" & to_ostring(bit_vector'(O"17")) & " bstr=" & to_bstring(bit_vector'("101"));
    wait;
  end process;
end;
