-- difftest: top=array_conversion_index exit=1 ref=hand
-- ref=hand: the expected text is yavs's wording of the run-time error (the references word it
-- differently).
-- The bounds of an array converted to an unconstrained array type must belong to the index
-- subtype of the target type (LRM 9.3.6); a violation is a run-time error (#379).
entity array_conversion_index is end;
architecture a of array_conversion_index is
  type ivec is array (integer range <>) of bit;
  function conv(lo : integer) return string is
    variable v : ivec(lo + 2 downto lo) := "101";
  begin
    return to_string(bit_vector(v));
  end function;
begin
  process
  begin
    report "conv(0) " & conv(0);
    report "conv(5) " & conv(5);
    report "conv(-1) " & conv(-1);
    report "not reached";
    wait;
  end process;
end;
