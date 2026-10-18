-- yavs-test: std=2008
-- expect: E1410 12:19 attribute dimension must be a locally static expression
-- LRM 16.2
entity e is end entity;
architecture a of e is
  signal m : bit_vector(0 to 3);
begin
  process
    variable d : integer := 1;
    variable n : integer;
  begin
    n := m'length(d);
    wait;
  end process;
end architecture;
