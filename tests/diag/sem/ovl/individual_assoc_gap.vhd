-- yavs-test: std=2008
-- expect: E1311 13:21 gap before v(9); note names elements 5 to 8
-- LRM 6.5.7.1
-- The elements of an unconstrained formal associated one by one form a contiguous
-- range: v(4) and v(9) leave 5 to 8 without an actual.
entity e is end entity;
architecture a of e is
  procedure swap(variable v : inout integer_vector) is begin end procedure;
begin
  process
    variable a, b : integer := 0;
  begin
    swap(v(4) => a, v(9) => b);
    wait;
  end process;
end architecture;
