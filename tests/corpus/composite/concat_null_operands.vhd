-- difftest: top=concat_null_operands std=2008
-- `&` with one null operand (LRM 9.2.5): the other operand's elements in the bounds of the
-- index subtype. (Two null operands are in tests/discrepancies/concat-null-operands.)
entity concat_null_operands is end;
architecture t of concat_null_operands is
  type iv is array (natural range <>) of integer;
  function info(s : string) return string is
  begin
    return "[" & s & "] " & integer'image(s'left) & " " & integer'image(s'right) & " "
      & integer'image(s'length);
  end;
  function info(v : iv) return string is
    variable r : string(1 to 3 + 2 * v'length);
  begin
    return integer'image(v'left) & " " & integer'image(v'right) & " " & integer'image(v'length);
  end;
  function ident(n : integer) return integer is
  begin
    return n;
  end;
begin
  process
    variable e1 : string(ident(5) to ident(4));
    variable e2 : string(ident(3) to ident(2));
    variable e3 : string(ident(10) to ident(9));
    variable a : string(7 to 9) := "xyz";
    variable i0 : iv(ident(8) to ident(7));
    variable i1 : iv(0 to -1);
    variable i2 : iv(4 to 6) := (1, 2, 3);
  begin
    report info(e1 & a);
    report info(a & e1);
    report info(e1 & a & e2);
    report info(e1 & 'q');
    report info('q' & e2);
    report info(i0 & i2);
    report info(i2 & i1);
    wait;
  end process;
end;
