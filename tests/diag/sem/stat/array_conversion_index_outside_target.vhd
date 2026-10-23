-- yavs-test: std=2008
-- expect: W1403 11:33 the bounds of the operand are not inside index subtype `NATURAL` of the target
-- LRM 9.3.6 (#465): a warning, the run-time check fails (the code may never run, L51)
entity e is end;
architecture a of e is
  type ivec is array (integer range <>) of bit;
begin
  process
    variable v : ivec(1 downto -1) := "101";
  begin
    report to_string(bit_vector(v));
    wait;
  end process;
end;
