-- difftest: top=cat1 std=2008 ref=hand
-- ref=hand: sim1 fails on a null & null concatenation; LRM 9.2.5 a: the result is the right operand.
entity cat1 is end;
architecture t of cat1 is
begin
  process
    variable a : string(5 to 4);
    variable b : string(10 downto 11);
    function l(s : string) return string is
    begin
      return integer'image(s'left) & " " & boolean'image(s'ascending);
    end;
  begin
    report l(a & b) & " | " & l(a & "xy");
    wait;
  end process;
end;
