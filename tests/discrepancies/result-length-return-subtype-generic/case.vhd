-- The value of a return statement must belong to the return type mark's subtype (LRM 10.13), here
-- sized by a generic: a value of another length is an error.
entity top is
  generic (g : integer := 4);
end;
architecture a of top is
  subtype nib is bit_vector(g - 1 downto 0);
  function cut(x : bit_vector; h : integer) return nib is
  begin
    return x(h downto 0);
  end;
begin
  process
    variable a : bit_vector(7 downto 0) := "10101010";
    variable r : nib;
  begin
    r := cut(a, 3);
    report "RES 1 " & integer'image(r'length);
    r := cut(a, 4);
    report "RES 2 " & integer'image(r'length);
    wait;
  end process;
end;
