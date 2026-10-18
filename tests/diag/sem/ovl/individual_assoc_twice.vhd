-- yavs-test: std=2008
-- expect: E1310 14:10 v(4 to 6) overlaps v(5)
-- LRM 6.5.7.1
-- Each element of an individually associated formal is associated exactly once,
-- here through an element and an overlapping slice.
entity e is end entity;
architecture a of e is
  procedure swap(variable v : inout integer_vector) is begin end procedure;
begin
  process
    variable a, b : integer := 0;
    variable c : integer_vector(4 to 6);
  begin
    swap(v(5) => a, v(4 to 6) => c);
    wait;
  end process;
end architecture;
