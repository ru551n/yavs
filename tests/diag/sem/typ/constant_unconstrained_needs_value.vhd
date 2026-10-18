-- yavs-test: std=2008
-- expect: E1212 8:14 constant `c` of unconstrained type needs an initial value
-- LRM 6.4.2.2
entity e is end entity;
architecture a of e is
begin
  process
    constant c : bit_vector;
  begin
    wait;
  end process;
end architecture;
