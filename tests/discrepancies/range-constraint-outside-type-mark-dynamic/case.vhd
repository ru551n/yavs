-- A range constraint whose bound is known only at run time and lies outside the range of its
-- type mark (LRM 5.2.1, 6.3).
entity top is end;
architecture a of top is
  function f(n : integer) return integer is
    variable v : positive range n to 3 := 3;
  begin
    return v;
  end function;
begin
  process
  begin
    report "f(2) = " & integer'image(f(2));
    report "f(0) = " & integer'image(f(0));
    wait;
  end process;
end;
