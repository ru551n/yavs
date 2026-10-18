-- yavs-test: std=2008
-- expect: E1224 8:14 constant `c` needs an initial value
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
