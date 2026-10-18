-- yavs-test: std=2008
-- expect: W1404 10:10 slice 2 to 5 is outside range 0 to 3
-- LRM 8.5
entity e is end entity;
architecture a of e is
  signal s : bit_vector(0 to 3);
  signal t : bit_vector(0 to 1);
begin
  process begin
    t <= s(2 to 5);
    wait;
  end process;
end architecture;
