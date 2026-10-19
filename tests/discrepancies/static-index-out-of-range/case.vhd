-- yavs-test: std=2008
-- expect: W1404 11:10 index 9 is outside range 0 to 3
-- LRM 8.4
entity e is end entity;
architecture a of e is
  signal s : bit_vector(0 to 3);
begin
  process
    variable b : bit;
  begin
    b := s(9);
    wait;
  end process;
end architecture;
