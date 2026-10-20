-- An index constraint with a bound known only at run time outside the index subtype of the
-- array type (LRM 5.3.2.2, 6.3).
entity top is end;
architecture a of top is
  function f(n : integer) return natural is
    variable b : bit_vector(1 downto n);
  begin
    return b'length;
  end function;
begin
  process
  begin
    report "f(0) = " & integer'image(f(0));
    report "f(-1) = " & integer'image(f(-1));
    wait;
  end process;
end;
