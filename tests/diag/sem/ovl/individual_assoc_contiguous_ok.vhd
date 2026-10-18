-- yavs-test: std=2008
-- expect: none
-- LRM 6.5.7.1
-- Near miss: elements (and slices in the port map) that tile a range without gap or overlap, in any order,
-- also for a port map and a record element array.
entity c is
  port (p : out bit_vector);
end entity;
architecture a of c is begin end architecture;
entity e is end entity;
architecture a of e is
  signal x, y : bit;
  signal z : bit_vector(1 downto 0);
  procedure swap(variable v : inout integer_vector) is begin end procedure;
begin
  u : entity work.c port map (p(2) => y, p(0 to 1) => z, p(3) => x);
  process
    variable a, b : integer := 0;
    variable c : integer := 0;
  begin
    swap(v(6) => b, v(5) => c, v(4) => a);
    swap(v(7) => a);
    wait;
  end process;
end architecture;
