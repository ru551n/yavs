-- Conversion to an unconstrained array type whose index subtype does not contain the
-- operand's static bounds (LRM 9.3.6).
entity top is end;
architecture a of top is
  type ivec is array (integer range <>) of bit;
begin
  process
    variable v : ivec(1 downto -1) := "101";
  begin
    report to_string(bit_vector(v));
    wait;
  end process;
end;
