-- difftest: top=strings_ops std=2008
entity strings_ops is end;
architecture t of strings_ops is
  function upper(s : string) return string is
    variable r : string(s'range);
  begin
    for i in s'range loop
      if s(i) >= 'a' and s(i) <= 'z' then r(i) := character'val(character'pos(s(i)) - 32); else r(i) := s(i); end if;
    end loop;
    return r;
  end;
  function rev(s : string) return string is variable r : string(1 to s'length); begin
    for i in s'range loop r(s'length - (i - s'low)) := s(i); end loop; return r;
  end;
begin
  process begin
    report upper("Hello, World 42");
    report rev("abcdef");
    report "slice " & upper("xyz")(2 to 3);
    assert "abc" & 'd' = "abcd" and 'a' & "bc" = "abc" severity failure;
    assert string'("abc") /= "abd" and string'("ab") < "abc" severity failure;
    report "slice2 " & upper("xyz")(1 to 1);
    wait;
  end process;
end;
