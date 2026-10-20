-- Conversion to an unconstrained array type whose index subtype does not contain the
-- operand's bounds, known only at run time (LRM 9.3.6).
entity top is end;
architecture a of top is
  type ivec is array (integer range <>) of bit;
  function f(lo : integer) return string is
    variable v : ivec(lo + 2 downto lo) := "101";
  begin
    return to_string(bit_vector(v));
  end function;
begin
  process
  begin
    report "f(0) = " & f(0);
    report "f(-1) = " & f(-1);
    wait;
  end process;
end;
